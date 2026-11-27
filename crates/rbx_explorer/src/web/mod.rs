//! The page side: the app's state, the screens' DOM, and the events that
//! drive them. One delegated listener per event type reads `data-act` off
//! whatever was tapped; the HTML itself comes from [`crate::ui`].
//!
//! The host page (`web/index.html`) supplies `window.rbxHost`, the few
//! things only it can do: open the platform's file picker, save bytes
//! (Android's Storage Access Framework, or a browser download), remember
//! recent files, tint the system bars. It calls back into the exported
//! `rbx_*` functions below.

mod dom;
mod editor;

use std::cell::RefCell;
use std::collections::HashMap;

use rbx_dom::Ref;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, Event, KeyboardEvent};

use crate::document::{Document, Format};
use crate::explorer::{all_scripts, path_names, Explorer};
use crate::history::History;
use crate::icons::symbols as s;
use crate::props::plural;
use crate::ui::{self, Recent, Tab, ROW_HEIGHT};
use editor::Editor;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = rbxHost)]
    fn pick();
    #[wasm_bindgen(js_namespace = rbxHost)]
    fn save(name: &str, bytes: &[u8], overwrite: bool);
    #[wasm_bindgen(js_namespace = rbxHost)]
    fn recents() -> String;
    #[wasm_bindgen(js_namespace = rbxHost, js_name = openRecent)]
    fn open_recent(id: &str);
    #[wasm_bindgen(js_namespace = rbxHost, js_name = setBars)]
    fn set_bars(dark: bool);
    #[wasm_bindgen(js_namespace = rbxHost)]
    fn copy(text: &str);
    #[wasm_bindgen(js_namespace = rbxHost)]
    fn exit();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Home,
    Loading,
    Doc,
    Editor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Sheet,
    Menu,
    Dialog,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    System,
    Light,
    Dark,
}

/// What to do once a save the user asked for finishes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterSave {
    Nothing,
    Close,
}

struct App {
    screen: Screen,
    overlay: Overlay,
    document: Option<Document>,
    /// Whether the host can write the file back where it came from.
    writable: bool,
    explorer: Explorer,
    scripts: Vec<Ref>,
    tab: Tab,
    script_query: String,
    edited_only: bool,
    selected: Option<Ref>,
    drawn_rows: (usize, usize),
    tree_scroll: i32,
    list_scroll: i32,
    editor: Option<Editor>,
    histories: HashMap<Ref, History>,
    font_size: f64,
    theme: Theme,
    save_as_format: Format,
    pending_save: Option<(String, Format, usize, AfterSave)>,
    snack_timer: Option<i32>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app, unless it is already busy (an event fired from
/// inside another handler is dropped rather than panicking).
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        guard.as_mut().map(f)
    })
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    let theme = match dom::storage_get("theme").as_deref() {
        Some("light") => Theme::Light,
        Some("dark") => Theme::Dark,
        _ => Theme::System,
    };
    let font_size = dom::storage_get("code-size")
        .and_then(|size| size.parse::<f64>().ok())
        .filter(|size| (9.0..=24.0).contains(size))
        .unwrap_or(13.0);
    let app = App {
        screen: Screen::Home,
        overlay: Overlay::None,
        document: None,
        writable: false,
        explorer: Explorer::default(),
        scripts: Vec::new(),
        tab: Tab::Tree,
        script_query: String::new(),
        edited_only: false,
        selected: None,
        drawn_rows: (usize::MAX, 0),
        tree_scroll: 0,
        list_scroll: 0,
        editor: None,
        histories: HashMap::new(),
        font_size,
        theme,
        save_as_format: Format::Binary,
        pending_save: None,
        snack_timer: None,
    };
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    install_listeners();
    with_app(|app| {
        app.apply_theme();
        app.render();
    });
}

// ---- calls from the host page -------------------------------------------

/// The host is reading a file: show the spinner.
#[wasm_bindgen]
pub fn rbx_loading(name: String) {
    with_app(|app| {
        app.close_overlay();
        app.screen = Screen::Loading;
        app.set_screen(&ui::loading(&name));
    });
}

/// A file arrived. `writable`: whether a plain "save" can write it back.
#[wasm_bindgen]
pub fn rbx_open(name: String, bytes: Vec<u8>, writable: bool) {
    with_app(|app| app.open(&name, &bytes, writable));
}

/// Reading a file failed before it reached us.
#[wasm_bindgen]
pub fn rbx_open_failed(message: String) {
    with_app(|app| {
        if app.screen == Screen::Loading {
            app.screen = if app.document.is_some() {
                Screen::Doc
            } else {
                Screen::Home
            };
            app.render();
        }
        app.snack(&format!("Не удалось открыть файл: {message}"));
    });
}

/// A save finished. `name` is the file's name now (a "save as" may rename);
/// `writable`: whether the document now lives in a file a plain "save" can
/// write back to (true on Android once it has been written anywhere, false
/// for a browser download).
#[wasm_bindgen]
pub fn rbx_saved(ok: bool, name: String, message: String, writable: bool) {
    with_app(|app| app.saved(ok, &name, &message, writable));
}

/// The system back gesture. Returns whether the app handled it (`false`
/// lets the host close the app).
#[wasm_bindgen]
pub fn rbx_back() -> bool {
    with_app(|app| app.back()).unwrap_or(true)
}

/// The host's recent-file list changed.
#[wasm_bindgen]
pub fn rbx_recents_changed() {
    with_app(|app| {
        if app.screen == Screen::Home {
            app.render();
        }
    });
}

// ---- events --------------------------------------------------------------

fn listen(
    target: &web_sys::EventTarget,
    kind: &str,
    capture: bool,
    f: impl FnMut(Event) + 'static,
) {
    let closure = Closure::<dyn FnMut(Event)>::new(f);
    let _ = target.add_event_listener_with_callback_and_bool(
        kind,
        closure.as_ref().unchecked_ref(),
        capture,
    );
    closure.forget();
}

fn act_target(event: &Event) -> Option<Element> {
    let target = event.target()?.dyn_into::<Element>().ok()?;
    target.closest("[data-act]").ok().flatten()
}

fn target_id(event: &Event) -> String {
    event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
        .map(|element| element.id())
        .unwrap_or_default()
}

fn install_listeners() {
    let document: web_sys::EventTarget = dom::document().into();
    let window: web_sys::EventTarget = dom::window().into();

    listen(&document, "click", false, |event| {
        if let Some(element) = act_target(&event) {
            let act = element.get_attribute("data-act").unwrap_or_default();
            with_app(|app| app.act(&act, &element));
        }
    });

    // Keys and the undo/redo buttons must not take focus off the code, or
    // every press would drop the soft keyboard.
    listen(&document, "mousedown", false, |event| {
        if let Some(element) = act_target(&event) {
            let act = element.get_attribute("data-act").unwrap_or_default();
            if matches!(act.as_str(), "key" | "undo" | "redo") {
                event.prevent_default();
            }
        }
    });

    listen(&document, "input", false, |event| {
        let id = target_id(&event);
        with_app(|app| app.on_input(&id));
    });

    listen(&document, "beforeinput", false, |event| {
        if target_id(&event) != "ta" {
            return;
        }
        let Some(input) = event.dyn_ref::<web_sys::InputEvent>() else {
            return;
        };
        let kind = input.input_type();
        let handled = with_app(|app| {
            let editor = app.editor.as_mut()?;
            match kind.as_str() {
                "insertLineBreak" | "insertParagraph" => editor.newline(),
                "historyUndo" => {
                    editor.undo();
                }
                "historyRedo" => {
                    editor.redo();
                }
                _ => return None,
            }
            app.after_edit();
            Some(())
        })
        .flatten();
        if handled.is_some() {
            event.prevent_default();
        }
    });

    listen(&document, "keydown", false, |event| {
        let Some(key) = event.dyn_ref::<KeyboardEvent>() else {
            return;
        };
        let id = target_id(&event);
        let handled = with_app(|app| app.on_key(key, &id)).unwrap_or(false);
        if handled {
            event.prevent_default();
        }
    });

    listen(&document, "scroll", true, |event| {
        let id = target_id(&event);
        with_app(|app| app.on_scroll(&id));
    });

    listen(&document, "selectionchange", false, |_| {
        with_app(|app| {
            if let Some(editor) = app.editor.as_mut() {
                let focused = dom::document()
                    .active_element()
                    .is_some_and(|element| element.id() == "ta");
                if focused {
                    editor.sync_selection();
                }
            }
        });
    });

    listen(&window, "resize", false, |_| {
        with_app(|app| {
            app.drawn_rows = (usize::MAX, 0);
            app.draw_tree();
            if let Some(editor) = app.editor.as_mut() {
                editor.draw(true);
                editor.reveal_caret();
            }
        });
    });

    if let Ok(Some(query)) = dom::window().match_media("(prefers-color-scheme: dark)") {
        listen(&query.into(), "change", false, |_| {
            with_app(|app| app.apply_theme());
        });
    }
}

// ---- the app ---------------------------------------------------------------

impl App {
    fn set_screen(&self, html: &str) {
        dom::set_html("app", html);
    }

    fn render(&mut self) {
        match self.screen {
            Screen::Home => {
                let recents = parse_recents(&recents());
                self.set_screen(&ui::home(&recents, self.theme_icon()));
            }
            Screen::Loading => {}
            Screen::Doc => self.render_doc(),
            Screen::Editor => self.render_editor(),
        }
    }

    fn theme_icon(&self) -> &'static str {
        match self.theme {
            Theme::System => s::CONTRAST,
            Theme::Light => s::LIGHT_MODE,
            Theme::Dark => s::DARK_MODE,
        }
    }

    fn apply_theme(&self) {
        let dark = match self.theme {
            Theme::Dark => true,
            Theme::Light => false,
            Theme::System => dom::window()
                .match_media("(prefers-color-scheme: dark)")
                .ok()
                .flatten()
                .is_some_and(|query| query.matches()),
        };
        if let Some(root) = dom::document().document_element() {
            let _ = root.set_attribute("data-theme", if dark { "dark" } else { "light" });
        }
        set_bars(dark);
    }

    fn cycle_theme(&mut self) {
        self.theme = match self.theme {
            Theme::System => Theme::Light,
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::System,
        };
        let (key, label) = match self.theme {
            Theme::System => ("system", "как в системе"),
            Theme::Light => ("light", "светлая"),
            Theme::Dark => ("dark", "тёмная"),
        };
        dom::storage_set("theme", key);
        self.apply_theme();
        if self.screen == Screen::Home {
            self.render();
        }
        self.snack(&format!("Тема: {label}"));
    }

    // ---- opening & saving ----------------------------------------------

    fn open(&mut self, name: &str, bytes: &[u8], writable: bool) {
        match Document::open(name, bytes) {
            Ok(document) => {
                self.explorer = Explorer::new(document.dom());
                self.scripts = all_scripts(document.dom());
                self.document = Some(document);
                self.writable = writable;
                self.tab = Tab::Tree;
                self.script_query.clear();
                self.edited_only = false;
                self.selected = None;
                self.editor = None;
                self.histories.clear();
                self.tree_scroll = 0;
                self.list_scroll = 0;
                self.screen = Screen::Doc;
                self.close_overlay();
                self.render();
            }
            Err(err) => {
                self.screen = if self.document.is_some() {
                    Screen::Doc
                } else {
                    Screen::Home
                };
                self.render();
                self.show_dialog(&ui::dialog(
                    Some(s::ERROR),
                    "Не удалось открыть",
                    &format!(
                        "«{}» не похож на место или модель Roblox.<br><br><span style=\"font-family:var(--font-mono);font-size:12px\">{}</span>",
                        ui::esc(name),
                        ui::esc(&err)
                    ),
                    &[("dismiss", "Понятно", "text")],
                ));
            }
        }
    }

    /// Saves in the file's own format, back where it came from when the
    /// host can, otherwise as a new file.
    fn save(&mut self, after: AfterSave) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let format = document.format();
        let name = if self.writable {
            document.name().to_string()
        } else {
            document.suggested_name(format)
        };
        self.encode_and_save(name, format, self.writable, after);
    }

    fn save_as(&mut self, format: Format) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let name = document.suggested_name(format);
        self.encode_and_save(name, format, false, AfterSave::Nothing);
    }

    fn encode_and_save(&mut self, name: String, format: Format, overwrite: bool, after: AfterSave) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        match document.encode(format) {
            Ok(bytes) => {
                self.pending_save = Some((name.clone(), format, bytes.len(), after));
                save(&name, &bytes, overwrite);
            }
            Err(err) => self.snack(&format!("Не удалось сохранить: {err}")),
        }
    }

    fn saved(&mut self, ok: bool, name: &str, message: &str, writable: bool) {
        let Some((pending_name, format, length, after)) = self.pending_save.take() else {
            return;
        };
        if !ok {
            if !message.is_empty() {
                self.snack(&format!("Не сохранено: {message}"));
            }
            return;
        }
        self.writable = writable;
        let name = if name.is_empty() {
            pending_name
        } else {
            name.to_string()
        };
        if let Some(document) = self.document.as_mut() {
            document.mark_saved(&name, format, length);
        }
        self.histories.clear();
        if after == AfterSave::Close {
            self.close_document();
            self.snack(&format!("Сохранено: {name}"));
            return;
        }
        self.refresh_dirty();
        match self.screen {
            Screen::Doc => self.render_doc(),
            Screen::Editor => dom::set_hidden("ed-dirty", true),
            _ => {}
        }
        self.snack(&format!("Сохранено: {name}"));
    }

    fn close_document(&mut self) {
        self.document = None;
        self.editor = None;
        self.histories.clear();
        self.scripts.clear();
        self.explorer = Explorer::default();
        self.screen = Screen::Home;
        self.close_overlay();
        self.render();
    }

    fn request_close(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !document.is_dirty() {
            self.close_document();
            return;
        }
        let count = document.edited_count();
        self.show_dialog(&ui::dialog(
            Some(s::SAVE),
            "Сохранить изменения?",
            &format!(
                "Изменено: {}. Если закрыть без сохранения, правки пропадут.",
                plural(count, "скрипт", "скрипта", "скриптов")
            ),
            &[
                ("dismiss", "Отмена", "text"),
                ("discard-close", "Не сохранять", "text danger"),
                ("save-close", "Сохранить", "filled"),
            ],
        ));
    }

    // ---- the document screen -------------------------------------------

    fn render_doc(&mut self) {
        let Some(document) = self.document.as_ref() else {
            self.screen = Screen::Home;
            self.render();
            return;
        };
        let query = match self.tab {
            Tab::Tree => self.explorer.query().to_string(),
            Tab::Scripts => self.script_query.clone(),
        };
        let html = ui::document_screen(document, self.tab, &query, self.scripts.len());
        self.set_screen(&html);
        match self.tab {
            Tab::Tree => {
                dom::set_html("chips", &ui::tree_chips(&self.explorer));
                self.drawn_rows = (usize::MAX, 0);
                self.size_tree();
                if let Some(tree) = dom::html_by_id("tree") {
                    tree.set_scroll_top(self.tree_scroll);
                }
                self.draw_tree();
            }
            Tab::Scripts => {
                self.draw_script_list();
                if let Some(list) = dom::html_by_id("slist") {
                    list.set_scroll_top(self.list_scroll);
                }
            }
        }
    }

    fn size_tree(&self) {
        let height = self.explorer.rows().len() as f64 * ROW_HEIGHT + 24.0;
        dom::set_style_id("spacer", "height", &format!("{height}px"));
    }

    /// Draws the tree rows in view, if they changed.
    fn draw_tree(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(tree) = dom::html_by_id("tree") else {
            return;
        };
        let top = f64::from(tree.scroll_top());
        let height = f64::from(tree.client_height()).max(400.0);
        let first = (top / ROW_HEIGHT).floor() as usize;
        let last = ((top + height) / ROW_HEIGHT).ceil() as usize;
        let (drawn_start, drawn_end) = self.drawn_rows;
        if drawn_start != usize::MAX && first >= drawn_start && last <= drawn_end {
            return;
        }
        let start = first.saturating_sub(12);
        let end = (last + 12).min(self.explorer.rows().len());
        self.drawn_rows = (start, end);
        let html = ui::tree_rows(document, &self.explorer, start..end, self.selected);
        dom::set_html("spacer", &html);
    }

    fn refresh_tree(&mut self) {
        dom::set_html("chips", &ui::tree_chips(&self.explorer));
        self.size_tree();
        self.drawn_rows = (usize::MAX, 0);
        self.draw_tree();
    }

    fn scroll_tree_to(&mut self, index: usize) {
        if let Some(tree) = dom::html_by_id("tree") {
            let height = f64::from(tree.client_height());
            let y = index as f64 * ROW_HEIGHT - height / 2.0 + ROW_HEIGHT / 2.0;
            tree.set_scroll_top(y.max(0.0) as i32);
        }
        self.drawn_rows = (usize::MAX, 0);
        self.draw_tree();
    }

    fn draw_script_list(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let query = self.script_query.trim().to_lowercase();
        let shown = ui::filter_scripts(document, &self.scripts, &query, self.edited_only);
        dom::set_html(
            "chips",
            &ui::script_chips(self.edited_only, shown.len(), self.scripts.len()),
        );
        dom::set_html("slist", &ui::script_list(document, &shown, &query));
    }

    fn switch_tab(&mut self, tab: Tab) {
        if self.tab == tab {
            // A second tap on the current tab scrolls it to the top.
            let id = if tab == Tab::Tree { "tree" } else { "slist" };
            if let Some(element) = dom::html_by_id(id) {
                element.set_scroll_top(0);
            }
            return;
        }
        self.remember_scroll();
        self.tab = tab;
        self.render_doc();
    }

    fn remember_scroll(&mut self) {
        if let Some(tree) = dom::html_by_id("tree") {
            self.tree_scroll = tree.scroll_top();
        }
        if let Some(list) = dom::html_by_id("slist") {
            self.list_scroll = list.scroll_top();
        }
    }

    fn select(&mut self, referent: Ref) {
        self.selected = Some(referent);
        self.drawn_rows = (usize::MAX, 0);
        self.draw_tree();
    }

    fn reveal(&mut self, referent: Ref) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let index = self.explorer.reveal(document.dom(), referent);
        self.selected = Some(referent);
        self.close_overlay();
        if self.screen != Screen::Doc || self.tab != Tab::Tree {
            self.leave_editor();
            self.tab = Tab::Tree;
            self.screen = Screen::Doc;
            self.render_doc();
        } else {
            self.refresh_tree();
            if let Some(input) = dom::input("q") {
                input.set_value("");
            }
            dom::set_hidden("clear-q", true);
        }
        if let Some(index) = index {
            self.scroll_tree_to(index);
        }
    }

    fn tap_row(&mut self, referent: Ref) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let dom_ = document.dom();
        let Some(instance) = dom_.get(referent) else {
            return;
        };
        let is_script = crate::document::is_script_class(instance.class());
        let has_children = !instance.children().is_empty();
        if is_script {
            self.selected = Some(referent);
            self.open_script(referent);
        } else if has_children && !self.explorer.is_filtered() {
            self.selected = Some(referent);
            self.explorer.toggle(document.dom(), referent);
            self.refresh_tree();
        } else {
            self.select(referent);
            self.open_sheet(referent);
        }
    }

    fn open_sheet(&mut self, referent: Ref) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        dom::set_html("overlay", &ui::sheet(document, referent));
        self.overlay = Overlay::Sheet;
    }

    fn close_overlay(&mut self) {
        if self.overlay != Overlay::None {
            dom::set_html("overlay", "");
            self.overlay = Overlay::None;
        }
    }

    fn show_dialog(&mut self, html: &str) {
        dom::set_html("overlay", html);
        self.overlay = Overlay::Dialog;
    }

    fn show_menu(&mut self, items: &[Option<ui::MenuItem<'_>>]) {
        let top = dom::by_id("app")
            .and_then(|app| app.query_selector(".top-bar").ok().flatten())
            .map_or(56.0, |bar| bar.get_bounding_client_rect().bottom() - 8.0);
        dom::set_html("overlay", &ui::menu(items, top));
        self.overlay = Overlay::Menu;
    }

    fn snack(&mut self, message: &str) {
        if let Some(timer) = self.snack_timer.take() {
            dom::clear_timeout(timer);
        }
        dom::set_html("snack", &ui::snackbar(message, None));
        let lift = if self.screen == Screen::Doc {
            "80px"
        } else {
            "0px"
        };
        dom::set_style_id("snack", "--snack-lift", lift);
        self.snack_timer = Some(dom::timeout(3200, || {
            dom::set_html("snack", "");
            with_app(|app| app.snack_timer = None);
        }));
    }

    fn refresh_dirty(&self) {
        if let Some(document) = self.document.as_ref() {
            dom::set_html("doc-titles", &ui::doc_titles(document));
        }
    }

    // ---- the editor ----------------------------------------------------

    fn open_script(&mut self, referent: Ref) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let Some(text) = document.source(referent) else {
            return;
        };
        self.remember_scroll();
        self.close_overlay();
        let history = self.histories.remove(&referent).unwrap_or_default();
        self.editor = Some(Editor::new(referent, text, history));
        self.screen = Screen::Editor;
        self.render_editor();
    }

    fn render_editor(&mut self) {
        let (Some(document), Some(editor)) = (self.document.as_ref(), self.editor.as_mut()) else {
            return;
        };
        dom::set_html("app", &ui::editor_screen(document, editor.referent));
        editor.mount(self.font_size);
        self.update_editor_buttons();
    }

    fn leave_editor(&mut self) {
        if let Some(editor) = self.editor.take() {
            self.histories.insert(editor.referent, editor.history);
        }
    }

    fn close_editor(&mut self) {
        self.leave_editor();
        self.screen = Screen::Doc;
        self.render_doc();
        if let Some(referent) = self.selected {
            if self.tab == Tab::Tree {
                if let Some(index) = self.explorer.index_of(referent) {
                    let visible = dom::html_by_id("tree").is_some_and(|tree| {
                        let top = f64::from(tree.scroll_top());
                        let y = index as f64 * ROW_HEIGHT;
                        y >= top && y <= top + f64::from(tree.client_height()) - ROW_HEIGHT
                    });
                    if !visible {
                        self.scroll_tree_to(index);
                    }
                }
            }
        }
    }

    /// After any change to the editor's text: store it in the document and
    /// update the marks that say it is edited.
    fn after_edit(&mut self) {
        let (Some(document), Some(editor)) = (self.document.as_mut(), self.editor.as_ref()) else {
            return;
        };
        document.set_source(editor.referent, &editor.text);
        let edited = document.is_edited(editor.referent);
        dom::set_hidden("ed-dirty", !edited);
        self.update_editor_buttons();
    }

    fn update_editor_buttons(&self) {
        if let Some(editor) = self.editor.as_ref() {
            dom::set_disabled("b-undo", !editor.history.can_undo());
            dom::set_disabled("b-redo", !editor.history.can_redo());
            dom::toggle_class("b-case", "selected", editor.case_sensitive);
            dom::toggle_class("b-find", "selected", editor.find_open);
        }
    }

    fn open_find(&mut self) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        if editor.find_open {
            if let Some(input) = dom::input("fq") {
                let _ = input.focus();
                input.select();
            }
            return;
        }
        dom::set_hidden("find", false);
        editor.open_find();
        if let Some(input) = dom::input("fq") {
            let _ = input.focus();
            input.select();
        }
        editor.draw(true);
        self.update_editor_buttons();
    }

    fn close_find(&mut self) {
        if let Some(editor) = self.editor.as_mut() {
            editor.close_find();
            dom::set_hidden("find", true);
            editor.draw(true);
        }
        self.update_editor_buttons();
    }

    fn set_font_size(&mut self, size: f64) {
        self.font_size = size.clamp(9.0, 24.0);
        dom::storage_set("code-size", &self.font_size.to_string());
        if let Some(editor) = self.editor.as_mut() {
            editor.apply_font(self.font_size);
        }
    }

    fn go_to_line_dialog(&mut self) {
        let lines = self.editor.as_ref().map_or(1, Editor::line_count);
        self.show_dialog(&ui::dialog(
            None,
            "Перейти к строке",
            &format!(
                "<label class=\"field\"><input id=\"goto\" type=\"number\" inputmode=\"numeric\" min=\"1\" max=\"{lines}\" placeholder=\"1 – {lines}\"><span class=\"lbl\">Номер строки</span></label>"
            ),
            &[("dismiss", "Отмена", "text"), ("goto-ok", "Перейти", "text")],
        ));
        if let Some(input) = dom::input("goto") {
            let _ = input.focus();
        }
    }

    fn go_to_line(&mut self) {
        let line = dom::input("goto")
            .and_then(|input| input.value().trim().parse::<usize>().ok())
            .unwrap_or(0);
        self.close_overlay();
        if line > 0 {
            if let Some(editor) = self.editor.as_mut() {
                editor.go_to_line(line);
                if let Some(textarea) = dom::textarea("ta") {
                    let _ = textarea.focus();
                }
            }
        }
    }

    fn revert_script(&mut self) {
        self.close_overlay();
        let (Some(document), Some(editor)) = (self.document.as_mut(), self.editor.as_mut()) else {
            return;
        };
        if let Some(original) = document.revert(editor.referent) {
            let length = editor.text.len();
            editor.replace(0, length, &original, (0, 0));
            document.set_source(editor.referent, &editor.text);
            dom::set_hidden("ed-dirty", true);
            self.update_editor_buttons();
            self.snack("Изменения скрипта отменены");
        }
    }

    // ---- dispatch ------------------------------------------------------

    fn act(&mut self, act: &str, element: &Element) {
        let referent = element
            .get_attribute("data-ref")
            .and_then(|value| value.parse::<u32>().ok())
            .map(Ref::new);
        match act {
            "pick" => pick(),
            "recent" => {
                if let Some(id) = element.get_attribute("data-id") {
                    open_recent(&id);
                }
            }
            "theme" => {
                self.close_overlay();
                self.cycle_theme();
            }
            "about" => self.show_dialog(&ui::dialog(
                Some(s::ACCOUNT_TREE),
                "RBX Explorer",
                "Просмотр мест и моделей Roblox (.rbxl, .rbxm, .rbxlx, .rbxmx): иерархия Explorer, свойства объектов и редактор скриптов Luau с подсветкой.<br><br>Написано на Rust поверх парсеров rbx-native. Иконки — Material Symbols (Apache 2.0).",
                &[("dismiss", "Закрыть", "text")],
            )),
            "dismiss" | "close-sheet" => self.close_overlay(),
            "close-doc" => self.request_close(),
            "discard-close" => self.close_document(),
            "save-close" => {
                self.close_overlay();
                self.save(AfterSave::Close);
            }
            "save" => {
                self.close_overlay();
                let dirty = self.document.as_ref().is_some_and(Document::is_dirty);
                if !dirty && self.writable {
                    self.snack("Изменений нет — файл уже сохранён");
                } else {
                    self.save(AfterSave::Nothing);
                }
            }
            "doc-menu" => {
                let all = self.explorer.all_services();
                self.show_menu(&[
                    Some(("save-as", s::SAVE_AS, "Сохранить копию…", "")),
                    None,
                    Some((
                        "all-services",
                        if all { s::VISIBILITY_OFF } else { s::VISIBILITY },
                        if all {
                            "Скрыть служебные"
                        } else {
                            "Показать все службы"
                        },
                        "",
                    )),
                    Some(("theme", s::CONTRAST, "Сменить тему", "")),
                    None,
                    Some(("close-doc", s::CLOSE, "Закрыть файл", "")),
                ]);
            }
            "save-as" => {
                self.save_as_format = self
                    .document
                    .as_ref()
                    .map_or(Format::Binary, Document::format);
                self.show_save_as();
            }
            "fmt-binary" => {
                self.save_as_format = Format::Binary;
                self.show_save_as();
            }
            "fmt-xml" => {
                self.save_as_format = Format::Xml;
                self.show_save_as();
            }
            "save-as-ok" => {
                self.close_overlay();
                self.save_as(self.save_as_format);
            }
            "all-services" => {
                self.close_overlay();
                if let Some(document) = self.document.as_ref() {
                    let all = !self.explorer.all_services();
                    self.explorer.set_all_services(document.dom(), all);
                    self.refresh_tree();
                }
            }
            "tab-tree" => self.switch_tab(Tab::Tree),
            "tab-scripts" => self.switch_tab(Tab::Scripts),
            "clear-q" => {
                if let Some(input) = dom::input("q") {
                    input.set_value("");
                    let _ = input.focus();
                }
                self.on_input("q");
            }
            "scripts-only" => {
                if let Some(document) = self.document.as_ref() {
                    let on = !self.explorer.scripts_only();
                    self.explorer.set_scripts_only(document.dom(), on);
                    if let Some(tree) = dom::html_by_id("tree") {
                        tree.set_scroll_top(0);
                    }
                    self.refresh_tree();
                }
            }
            "collapse-all" => {
                if let Some(document) = self.document.as_ref() {
                    self.explorer.collapse_all(document.dom());
                    self.refresh_tree();
                }
            }
            "edited-only" => {
                self.edited_only = !self.edited_only;
                self.draw_script_list();
            }
            "row" => {
                if let Some(referent) = referent {
                    self.tap_row(referent);
                }
            }
            "twisty" => {
                if let (Some(referent), Some(document)) = (referent, self.document.as_ref()) {
                    self.explorer.toggle(document.dom(), referent);
                    self.refresh_tree();
                }
            }
            "info" => {
                if let Some(referent) = referent {
                    if self.screen == Screen::Doc && self.tab == Tab::Tree {
                        self.select(referent);
                    }
                    self.open_sheet(referent);
                }
            }
            "open-script" => {
                if let Some(referent) = referent {
                    self.selected = Some(referent);
                    self.open_script(referent);
                }
            }
            "reveal" => {
                if let Some(referent) = referent {
                    self.reveal(referent);
                }
            }
            "copy-path" => {
                if let (Some(referent), Some(document)) = (referent, self.document.as_ref()) {
                    let path = path_names(document.dom(), referent);
                    let mut text = String::new();
                    for (index, name) in path.iter().enumerate() {
                        let simple = !name.is_empty()
                            && name.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                            && !name.starts_with(|ch: char| ch.is_ascii_digit());
                        if index == 0 {
                            text.push_str(&format!("game:GetService(\"{name}\")"));
                        } else if simple {
                            text.push('.');
                            text.push_str(name);
                        } else {
                            text.push_str(&format!("[\"{}\"]", name.replace('"', "\\\"")));
                        }
                    }
                    if document.kind() == crate::document::Kind::Model {
                        text = path.join(".");
                    }
                    copy(&text);
                    self.snack("Путь скопирован");
                }
            }
            "close-editor" => self.close_editor(),
            "undo" | "redo" => {
                if let Some(editor) = self.editor.as_mut() {
                    if act == "undo" {
                        editor.undo();
                    } else {
                        editor.redo();
                    }
                }
                self.after_edit();
            }
            "find" => {
                let open = self.editor.as_ref().is_some_and(|editor| editor.find_open);
                if open {
                    self.close_find();
                } else {
                    self.open_find();
                }
            }
            "find-close" => self.close_find(),
            "find-next" | "find-prev" => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.find_step(act == "find-next");
                }
            }
            "find-case" => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.toggle_case();
                }
                self.update_editor_buttons();
            }
            "replace-one" | "replace-all" => {
                let with = dom::input("fr").map(|input| input.value()).unwrap_or_default();
                if let Some(editor) = self.editor.as_mut() {
                    if act == "replace-one" {
                        editor.replace_one(&with);
                    } else {
                        let count = editor.replace_every(&with);
                        self.after_edit();
                        self.snack(&format!(
                            "Заменено: {}",
                            plural(count, "совпадение", "совпадения", "совпадений")
                        ));
                        return;
                    }
                }
                self.after_edit();
            }
            "key" => {
                if let Some(editor) = self.editor.as_mut() {
                    if let Some(text) = element.get_attribute("data-insert") {
                        editor.insert(&text);
                    } else {
                        match element.get_attribute("data-key").as_deref() {
                            Some("tab") => editor.tab(false),
                            Some("left") => editor.step(false),
                            Some("right") => editor.step(true),
                            _ => {}
                        }
                    }
                }
                self.after_edit();
            }
            "editor-menu" => {
                let size = self.font_size;
                let size_label = format!("{size} px");
                self.show_menu(&[
                    Some(("goto", s::FORMAT_LIST_NUMBERED, "Перейти к строке…", "")),
                    Some(("font-up", s::TEXT_INCREASE, "Крупнее", &size_label)),
                    Some(("font-down", s::TEXT_DECREASE, "Мельче", "")),
                    None,
                    Some(("editor-info", s::INFO, "Свойства скрипта", "")),
                    Some(("editor-reveal", s::ACCOUNT_TREE, "Показать в дереве", "")),
                    Some(("revert", s::RESTART, "Отменить все правки", "")),
                    None,
                    Some(("save", s::SAVE, "Сохранить файл", "")),
                ]);
            }
            "goto" => self.go_to_line_dialog(),
            "goto-ok" => self.go_to_line(),
            "font-up" | "font-down" => {
                self.close_overlay();
                let step = if act == "font-up" { 1.0 } else { -1.0 };
                self.set_font_size(self.font_size + step);
                self.snack(&format!("Размер кода: {} px", self.font_size));
            }
            "editor-info" => {
                self.close_overlay();
                if let Some(referent) = self.editor.as_ref().map(|editor| editor.referent) {
                    self.open_sheet(referent);
                }
            }
            "editor-reveal" => {
                if let Some(referent) = self.editor.as_ref().map(|editor| editor.referent) {
                    self.reveal(referent);
                }
            }
            "revert" => {
                let edited = match (self.document.as_ref(), self.editor.as_ref()) {
                    (Some(document), Some(editor)) => document.is_edited(editor.referent),
                    _ => false,
                };
                if edited {
                    self.show_dialog(&ui::dialog(
                        Some(s::RESTART),
                        "Отменить все правки?",
                        "Скрипт вернётся к тексту из файла. Это действие тоже можно отменить кнопкой «Отменить».",
                        &[("dismiss", "Нет", "text"), ("revert-ok", "Отменить правки", "text danger")],
                    ));
                } else {
                    self.close_overlay();
                    self.snack("Скрипт не изменён");
                }
            }
            "revert-ok" => self.revert_script(),
            _ => {}
        }
    }

    fn show_save_as(&mut self) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let option = |act: &str, on: bool, label: &str, name: String| {
            format!(
                "<button class=\"opt st\" data-act=\"{act}\"><span class=\"radio{}\"></span><span><div>{label}</div><div class=\"t-body-s muted\">{}</div></span></button>",
                if on { " on" } else { "" },
                ui::esc(&name)
            )
        };
        let body = format!(
            "{}{}",
            option(
                "fmt-binary",
                self.save_as_format == Format::Binary,
                "Бинарный",
                document.suggested_name(Format::Binary)
            ),
            option(
                "fmt-xml",
                self.save_as_format == Format::Xml,
                "XML",
                document.suggested_name(Format::Xml)
            )
        );
        self.show_dialog(&ui::dialog(
            Some(s::SAVE_AS),
            "Сохранить копию",
            &body,
            &[
                ("dismiss", "Отмена", "text"),
                ("save-as-ok", "Сохранить", "text"),
            ],
        ));
    }

    fn on_input(&mut self, id: &str) {
        match id {
            "q" => {
                let value = dom::input("q")
                    .map(|input| input.value())
                    .unwrap_or_default();
                dom::set_hidden("clear-q", value.is_empty());
                match self.tab {
                    Tab::Tree => {
                        if let Some(document) = self.document.as_ref() {
                            self.explorer.set_query(document.dom(), &value);
                            if let Some(tree) = dom::html_by_id("tree") {
                                tree.set_scroll_top(0);
                            }
                            self.refresh_tree();
                        }
                    }
                    Tab::Scripts => {
                        self.script_query = value;
                        self.draw_script_list();
                    }
                }
            }
            "ta" => {
                let changed = self.editor.as_mut().is_some_and(Editor::on_input);
                if changed {
                    self.after_edit();
                }
            }
            "fq" => {
                let value = dom::input("fq")
                    .map(|input| input.value())
                    .unwrap_or_default();
                if let Some(editor) = self.editor.as_mut() {
                    editor.set_query(&value);
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, key: &KeyboardEvent, id: &str) -> bool {
        let name = key.key();
        let ctrl = key.ctrl_key() || key.meta_key();
        if name == "Escape" {
            return self.back();
        }
        if ctrl && name.eq_ignore_ascii_case("s") && self.document.is_some() {
            self.save(AfterSave::Nothing);
            return true;
        }
        match id {
            "ta" => {
                let Some(editor) = self.editor.as_mut() else {
                    return false;
                };
                match name.as_str() {
                    "Tab" => editor.tab(key.shift_key()),
                    "z" | "Z" | "я" | "Я" if ctrl => {
                        if key.shift_key() {
                            editor.redo();
                        } else {
                            editor.undo();
                        }
                    }
                    "y" | "Y" | "н" | "Н" if ctrl => {
                        editor.redo();
                    }
                    "f" | "F" | "а" | "А" if ctrl => {
                        self.open_find();
                        return true;
                    }
                    _ => return false,
                }
                self.after_edit();
                true
            }
            "fq" if name == "Enter" => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.find_step(!key.shift_key());
                }
                true
            }
            "goto" if name == "Enter" => {
                self.go_to_line();
                true
            }
            "q" if name == "Enter" => {
                if let Some(input) = dom::input("q") {
                    let _ = input.blur();
                }
                true
            }
            _ => false,
        }
    }

    fn on_scroll(&mut self, id: &str) {
        match id {
            "tree" => {
                self.draw_tree();
                let scrolled = dom::html_by_id("tree").is_some_and(|tree| tree.scroll_top() > 0);
                dom::toggle_class("toolbar", "scrolled", scrolled);
            }
            "slist" => {
                let scrolled = dom::html_by_id("slist").is_some_and(|list| list.scroll_top() > 0);
                dom::toggle_class("toolbar", "scrolled", scrolled);
            }
            "code" => {
                if let Some(editor) = self.editor.as_mut() {
                    editor.draw(false);
                }
            }
            _ => {}
        }
    }

    /// Back: close what is on top. `false` when there is nothing left to
    /// close (the host then leaves the app).
    fn back(&mut self) -> bool {
        if self.overlay != Overlay::None {
            self.close_overlay();
            return true;
        }
        match self.screen {
            Screen::Editor => {
                if self.editor.as_ref().is_some_and(|editor| editor.find_open) {
                    self.close_find();
                } else {
                    self.close_editor();
                }
                true
            }
            Screen::Doc => {
                if self.explorer.is_filtered() && self.tab == Tab::Tree {
                    if let Some(document) = self.document.as_ref() {
                        self.explorer.set_query(document.dom(), "");
                        self.explorer.set_scripts_only(document.dom(), false);
                    }
                    self.render_doc();
                } else {
                    self.request_close();
                }
                true
            }
            Screen::Loading => true,
            Screen::Home => {
                exit();
                false
            }
        }
    }
}

/// The host's recents, as JSON: `[{"id", "name", "size", "time"}]`.
fn parse_recents(json: &str) -> Vec<Recent> {
    let Ok(value) = js_sys::JSON::parse(json) else {
        return Vec::new();
    };
    let Some(array) = value.dyn_ref::<js_sys::Array>() else {
        return Vec::new();
    };
    let now = js_sys::Date::new_0();
    let today = js_sys::Date::new_with_year_month_day(
        now.get_full_year(),
        now.get_month() as i32,
        now.get_date() as i32,
    )
    .get_time();
    let get = |item: &JsValue, key: &str| js_sys::Reflect::get(item, &JsValue::from_str(key)).ok();
    array
        .iter()
        .filter_map(|item| {
            let id = get(&item, "id")?.as_string()?;
            let name = get(&item, "name")?.as_string()?;
            let size = get(&item, "size").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let time = get(&item, "time").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let format = if name.to_ascii_lowercase().ends_with('x') {
                "XML"
            } else {
                "бинарный"
            };
            let kind = if name.to_ascii_lowercase().contains(".rbxl") {
                "Место"
            } else {
                "Модель"
            };
            let when = if time >= today {
                "Сегодня".to_string()
            } else if time >= today - 86_400_000.0 {
                "Вчера".to_string()
            } else {
                let date = js_sys::Date::new(&JsValue::from_f64(time));
                format!("{:02}.{:02}", date.get_date(), date.get_month() + 1)
            };
            Some(Recent {
                id,
                name,
                detail: format!(
                    "{kind} · {format} · {}",
                    crate::props::human_bytes(size as usize)
                ),
                when,
            })
        })
        .collect()
}
