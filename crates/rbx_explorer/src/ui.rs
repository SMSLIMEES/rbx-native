//! The HTML every screen is made of, written by Rust so the markup, the
//! strings and the logic deciding what shows live in one tested place. The
//! classes are `web/style.css`'s; interactive elements carry `data-act` (and
//! `data-ref`, an instance's referent) for the one delegated click handler
//! in `web`.

use std::fmt::Write;

use rbx_dom::{Ref, WeakDom};

use crate::document::{script_kind, Document, Format, Kind, ScriptKind};
use crate::explorer::{find_folded, path_names, Explorer, Row};
use crate::highlight::{push_escaped, render_line, State};
use crate::icons::{class_icon, icon, svg, symbols as s};
use crate::props::{self, group_digits, human_bytes, plural, Value};

/// Height of a tree row, matching `.node` in the stylesheet.
pub const ROW_HEIGHT: f64 = 48.0;

/// `text` escaped for HTML content and attributes.
pub fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

fn icon_sm(path: &str) -> String {
    svg(path, "i sm")
}

fn icon_btn(act: &str, path: &str, label: &str, extra: &str) -> String {
    format!(
        "<button class=\"icon-btn st{extra}\" data-act=\"{act}\" aria-label=\"{label}\" title=\"{label}\">{}</button>",
        icon(path)
    )
}

/// "Место · бинарный · 4,2 МБ".
pub fn describe_file(kind: Kind, format: Format, bytes: usize) -> String {
    let kind = match kind {
        Kind::Place => "Место",
        Kind::Model => "Модель",
    };
    let format = match format {
        Format::Binary => "бинарный",
        Format::Xml => "XML",
    };
    format!("{kind} · {format} · {}", human_bytes(bytes))
}

/// A file the host remembers (Android keeps the document's permission).
pub struct Recent {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub when: String,
}

fn file_icon(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".rbxl") || lower.ends_with(".rbxlx") {
        s::C_PUBLIC
    } else {
        s::C_CUBE
    }
}

/// The start screen.
pub fn home(recents: &[Recent], theme_icon: &str) -> String {
    let mut out = String::new();
    out.push_str("<section class=\"screen\" data-screen=\"home\"><header class=\"top-bar medium\"><div class=\"row\">");
    out.push_str(&icon_btn("theme", theme_icon, "Тема", ""));
    out.push_str(&icon_btn("about", s::INFO, "О приложении", ""));
    out.push_str("</div><div class=\"headline\">RBX Explorer</div></header>");
    out.push_str("<div class=\"scroll\"><div class=\"home\"><div class=\"hero\">");
    let _ = write!(
        out,
        "<div class=\"art\">{}</div>",
        svg(s::ACCOUNT_TREE_FILL, "")
    );
    out.push_str("<h2>Открыть место или модель</h2>");
    out.push_str("<p>Иерархия Explorer, свойства объектов и редактор скриптов Luau — прямо на телефоне. Без 3D, только структура и код.</p>");
    out.push_str("<div class=\"chips\"><span class=\"chip\">.rbxl</span><span class=\"chip\">.rbxm</span><span class=\"chip\">.rbxlx</span><span class=\"chip\">.rbxmx</span></div>");
    let _ = write!(
        out,
        "<div class=\"actions\"><button class=\"btn filled large has-icon st\" data-act=\"pick\">{}Выбрать файл</button></div></div>",
        icon(s::FOLDER_OPEN)
    );
    if !recents.is_empty() {
        out.push_str("<div class=\"section-title\">Недавние</div><div class=\"list card\">");
        for recent in recents {
            let _ = write!(
                out,
                "<button class=\"li two st\" data-act=\"recent\" data-id=\"{}\"><span class=\"lead\">{}</span><span class=\"txt\"><div class=\"head\">{}</div><div class=\"sup\">{}</div></span><span class=\"trail\">{}</span></button>",
                esc(&recent.id),
                icon(file_icon(&recent.name)),
                esc(&recent.name),
                esc(&recent.detail),
                esc(&recent.when)
            );
        }
        out.push_str("</div>");
    } else {
        let _ = write!(
            out,
            "<div class=\"list card\"><div class=\"li two\"><span class=\"lead\">{}</span><span class=\"txt\"><div class=\"head\">Совет</div><div class=\"sup\" style=\"white-space:normal\">Файл можно открыть и из файлового менеджера, Telegram или Диска: «Открыть с помощью» → RBX Explorer.</div></span></div></div>",
            icon(s::HISTORY)
        );
    }
    out.push_str("</div></div></section>");
    out
}

/// The spinner shown while a file parses.
pub fn loading(name: &str) -> String {
    format!(
        "<section class=\"screen\"><div class=\"loading\"><svg class=\"spinner\" viewBox=\"0 0 48 48\"><circle cx=\"24\" cy=\"24\" r=\"20\"/></svg><div>Открываю {}…</div></div></section>",
        esc(name)
    )
}

/// Which tab of the document screen is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Tree,
    Scripts,
}

/// The title bar text of the document screen.
pub fn doc_titles(document: &Document) -> String {
    let dirty = if document.is_dirty() {
        "<span class=\"dirty-dot\"></span>"
    } else {
        ""
    };
    let kind = match document.kind() {
        Kind::Place => "Место",
        Kind::Model => "Модель",
    };
    format!(
        "<div class=\"title\">{}{dirty}</div><div class=\"subtitle\">{kind} · {}</div>",
        esc(document.name()),
        plural(document.instance_count(), "объект", "объекта", "объектов")
    )
}

/// The document screen's frame; the rows and list are filled in by
/// [`tree_rows`] and [`script_list`].
pub fn document_screen(document: &Document, tab: Tab, query: &str, script_count: usize) -> String {
    let mut out = String::new();
    out.push_str(
        "<section class=\"screen\" data-screen=\"doc\"><header class=\"top-bar has-sub\">",
    );
    out.push_str(&icon_btn(
        "close-doc",
        s::ARROW_BACK,
        "Закрыть",
        " on-surface",
    ));
    let _ = write!(
        out,
        "<div class=\"titles\" id=\"doc-titles\">{}</div>",
        doc_titles(document)
    );
    out.push_str(&icon_btn("save", s::SAVE, "Сохранить", ""));
    out.push_str(&icon_btn("doc-menu", s::MORE_VERT, "Ещё", ""));
    out.push_str("</header><div class=\"toolbar\" id=\"toolbar\">");
    let placeholder = match tab {
        Tab::Tree => "Поиск по имени или классу",
        Tab::Scripts => "Поиск скрипта",
    };
    let _ = write!(
        out,
        "<label class=\"search\">{}<input id=\"q\" type=\"search\" autocomplete=\"off\" autocapitalize=\"off\" spellcheck=\"false\" enterkeyhint=\"search\" placeholder=\"{placeholder}\" value=\"{}\">",
        svg(s::SEARCH, "i lead-i"),
        esc(query)
    );
    let _ = write!(
        out,
        "<button class=\"icon-btn st\" data-act=\"clear-q\" id=\"clear-q\"{}>{}</button></label>",
        if query.is_empty() { " hidden" } else { "" },
        icon(s::CLOSE)
    );
    out.push_str("<div class=\"chip-row\" id=\"chips\"></div></div>");
    match tab {
        Tab::Tree => out.push_str(
            "<div class=\"tree\" id=\"tree\"><div class=\"spacer\" id=\"spacer\"></div></div>",
        ),
        Tab::Scripts => out.push_str("<div class=\"scroll\" id=\"slist\"></div>"),
    }
    let (tree_active, scripts_active) = match tab {
        Tab::Tree => (" active", ""),
        Tab::Scripts => ("", " active"),
    };
    let _ = write!(
        out,
        "<nav class=\"nav-bar\"><button class=\"nav-item{tree_active}\" data-act=\"tab-tree\"><span class=\"ind\">{}</span>Проводник</button>",
        icon(if tab == Tab::Tree {
            s::ACCOUNT_TREE_FILL
        } else {
            s::ACCOUNT_TREE
        })
    );
    let edited = document.edited_count();
    let badge = if edited > 0 {
        format!("<span class=\"badge\">{edited}</span>")
    } else {
        String::new()
    };
    let _ = write!(
        out,
        "<button class=\"nav-item{scripts_active}\" data-act=\"tab-scripts\"><span class=\"ind\" id=\"scripts-ind\">{}{badge}</span>Скрипты · {}</button></nav></section>",
        icon(if tab == Tab::Scripts {
            s::CODE_BLOCKS_FILL
        } else {
            s::CODE_BLOCKS
        }),
        group_digits(script_count)
    );
    out
}

/// The filter chips under the search bar.
pub fn tree_chips(explorer: &Explorer) -> String {
    let mut out = String::new();
    let scripts = if explorer.scripts_only() {
        format!(
            "<button class=\"chip selected st\" data-act=\"scripts-only\">{}Скрипты</button>",
            icon(s::CHECK)
        )
    } else {
        "<button class=\"chip st\" data-act=\"scripts-only\">Скрипты</button>".to_string()
    };
    out.push_str(&scripts);
    if !explorer.is_filtered() {
        let _ = write!(
            out,
            "<button class=\"chip st\" data-act=\"collapse-all\">{}Свернуть всё</button>",
            icon(s::UNFOLD_LESS)
        );
    }
    let stat = if explorer.is_filtered() {
        match explorer.match_count() {
            0 => "Ничего не найдено".to_string(),
            n => format!(
                "{} · {}",
                plural(n, "совпадение", "совпадения", "совпадений"),
                plural(explorer.rows().len(), "строка", "строки", "строк")
            ),
        }
    } else {
        plural(explorer.rows().len(), "строка", "строки", "строк")
    };
    let _ = write!(out, "<span class=\"stat\">{stat}</span>");
    out
}

fn script_tag(kind: Option<ScriptKind>) -> &'static str {
    match kind {
        Some(ScriptKind::Server) => "<span class=\"tag server\">Server</span>",
        Some(ScriptKind::Client) => "<span class=\"tag client\">Client</span>",
        Some(ScriptKind::Module) => "<span class=\"tag module\">Module</span>",
        None => "",
    }
}

/// `name` with the part matching `query` (lowercase) marked.
fn marked_name(name: &str, query: &str) -> String {
    match find_folded(name, query) {
        Some((start, end)) => format!(
            "{}<mark>{}</mark>{}",
            esc(&name[..start]),
            esc(&name[start..end]),
            esc(&name[end..])
        ),
        None => esc(name),
    }
}

/// The tree rows `range` of `explorer`, absolutely positioned.
pub fn tree_rows(
    document: &Document,
    explorer: &Explorer,
    range: std::ops::Range<usize>,
    selected: Option<Ref>,
) -> String {
    let dom = document.dom();
    let rows = explorer.rows();
    let mut out = String::with_capacity(range.len() * 900);
    let start = range.start.min(rows.len());
    let end = range.end.min(rows.len());
    for (index, row) in rows[start..end]
        .iter()
        .enumerate()
        .map(|(i, row)| (i + start, row))
    {
        let row: &Row = row;
        let Some(instance) = dom.get(row.referent) else {
            continue;
        };
        let referent = row.referent.value();
        let class = instance.class();
        let (glyph, family) = class_icon(class);
        let sel = if selected == Some(row.referent) {
            " sel"
        } else {
            ""
        };
        let dim = if explorer.is_filtered() && !row.matched {
            " style=\"opacity:.62\""
        } else {
            ""
        };
        let _ = write!(
            out,
            "<div class=\"node st{sel}\" style=\"top:{}px\" data-act=\"row\" data-ref=\"{referent}\"><span class=\"pill\"></span><span class=\"guides\" style=\"width:{}px\"></span>",
            index as f64 * ROW_HEIGHT,
            u32::from(row.depth) * 20
        );
        if row.has_children && !explorer.is_filtered() {
            let _ = write!(
                out,
                "<button class=\"tw st{}\" data-act=\"twisty\" data-ref=\"{referent}\" aria-label=\"Развернуть\">{}</button>",
                if row.expanded { " open" } else { "" },
                icon_sm(s::CHEVRON_RIGHT)
            );
        } else if row.has_children {
            let _ = write!(
                out,
                "<span class=\"tw{}\">{}</span>",
                if row.expanded { " open" } else { "" },
                icon_sm(s::CHEVRON_RIGHT)
            );
        } else {
            out.push_str("<span class=\"tw leaf\"></span>");
        }
        let _ = write!(
            out,
            "<span class=\"ic {}\"{dim}>{}</span><span class=\"nm\"{dim}><span class=\"n\">{}</span>",
            family.css(),
            icon(glyph),
            marked_name(instance.name(), explorer.query())
        );
        if !crate::document::is_script_class(class)
            && !crate::explorer::services::is_known_service(class)
            && instance.name() != class
        {
            let _ = write!(out, "<span class=\"c\">{}</span>", esc(class));
        }
        if document.is_edited(row.referent) {
            out.push_str("<span class=\"tag edited\"></span>");
        }
        out.push_str("</span>");
        out.push_str(script_tag(script_kind(dom, row.referent)));
        let _ = write!(
            out,
            "<button class=\"info st\" data-act=\"info\" data-ref=\"{referent}\" aria-label=\"Свойства\">{}</button></div>",
            icon_sm(s::INFO)
        );
    }
    out
}

/// Chips of the Scripts tab.
pub fn script_chips(edited_only: bool, shown: usize, total: usize) -> String {
    let edited = if edited_only {
        format!(
            "<button class=\"chip selected st\" data-act=\"edited-only\">{}Изменённые</button>",
            icon(s::CHECK)
        )
    } else {
        "<button class=\"chip st\" data-act=\"edited-only\">Изменённые</button>".to_string()
    };
    let stat = if shown == total {
        plural(total, "скрипт", "скрипта", "скриптов")
    } else {
        format!("{} из {}", group_digits(shown), group_digits(total))
    };
    format!("{edited}<span class=\"stat\">{stat}</span>")
}

/// The Scripts tab's list.
pub fn script_list(document: &Document, scripts: &[Ref], query: &str) -> String {
    let dom = document.dom();
    if scripts.is_empty() {
        let (title, text) = if query.is_empty() {
            (
                "Скриптов нет",
                "В этом файле нет Script, LocalScript или ModuleScript.",
            )
        } else {
            ("Ничего не найдено", "Попробуйте другое имя.")
        };
        return format!(
            "<div class=\"empty\"><div class=\"badge-icon\">{}</div><div class=\"t\">{title}</div><div>{text}</div></div>",
            icon(s::CODE_BLOCKS)
        );
    }
    let mut out = String::from("<div class=\"list\" style=\"padding:4px 0 16px\">");
    for referent in scripts {
        let Some(instance) = dom.get(*referent) else {
            continue;
        };
        let (glyph, family) = class_icon(instance.class());
        let mut path = path_names(dom, *referent);
        path.pop();
        let lines = document
            .source(*referent)
            .map(|text| crate::textops::line_count(&text))
            .unwrap_or(0);
        let edited = if document.is_edited(*referent) {
            "<span class=\"tag edited\">изменён</span>"
        } else {
            ""
        };
        let _ = write!(
            out,
            "<button class=\"li two st\" data-act=\"open-script\" data-ref=\"{}\"><span class=\"lead {}\">{}</span><span class=\"txt\"><div class=\"head\" style=\"display:flex;gap:8px;align-items:center\"><span style=\"overflow:hidden;text-overflow:ellipsis\">{}</span>{edited}</div><div class=\"sup\">{} · {}</div></span>{}</button>",
            referent.value(),
            family.css(),
            icon(glyph),
            marked_name(instance.name(), query),
            esc(&path.join(" › ")),
            plural(lines, "строка", "строки", "строк"),
            script_tag(script_kind(dom, *referent))
        );
    }
    out.push_str("</div>");
    out
}

/// Scripts matching `query` (lowercase), edited ones only if asked.
pub fn filter_scripts(
    document: &Document,
    scripts: &[Ref],
    query: &str,
    edited_only: bool,
) -> Vec<Ref> {
    let dom = document.dom();
    scripts
        .iter()
        .copied()
        .filter(|referent| !edited_only || document.is_edited(*referent))
        .filter(|referent| {
            query.is_empty()
                || dom
                    .get(*referent)
                    .is_some_and(|instance| find_folded(instance.name(), query).is_some())
        })
        .collect()
}

/// The Properties bottom sheet of `referent`.
pub fn sheet(document: &Document, referent: Ref) -> String {
    let dom: &WeakDom = document.dom();
    let Some(instance) = dom.get(referent) else {
        return String::new();
    };
    let (glyph, family) = class_icon(instance.class());
    let mut out = String::new();
    out.push_str("<div class=\"scrim\" data-act=\"close-sheet\"></div><div class=\"sheet\" id=\"sheet\"><div class=\"handle\"></div>");
    let _ = write!(
        out,
        "<div class=\"sheet-head\"><span class=\"ic {}\">{}</span><div class=\"txt\"><div class=\"ttl\">{}</div><div class=\"sub\">{}</div></div>{}</div>",
        family.css(),
        icon(glyph),
        esc(instance.name()),
        esc(instance.class()),
        icon_btn("close-sheet", s::CLOSE, "Закрыть", "")
    );
    out.push_str("<div class=\"body\"><div class=\"path\">");
    let path = path_names(dom, referent);
    for (index, name) in path.iter().enumerate() {
        if index > 0 {
            out.push_str(&svg(s::CHEVRON_RIGHT, "i"));
        }
        let _ = write!(out, "<span>{}</span>", esc(name));
    }
    out.push_str("</div>");
    let is_script = script_kind(dom, referent).is_some();
    let children = instance.children().len();
    out.push_str("<div class=\"actions\">");
    if is_script {
        let _ = write!(
            out,
            "<button class=\"btn filled has-icon st\" data-act=\"open-script\" data-ref=\"{}\">{}Открыть скрипт</button>",
            referent.value(),
            icon(s::CODE)
        );
    }
    let _ = write!(
        out,
        "<button class=\"btn tonal has-icon st\" data-act=\"reveal\" data-ref=\"{}\">{}Показать в дереве</button>",
        referent.value(),
        icon(s::ACCOUNT_TREE)
    );
    let _ = write!(
        out,
        "<button class=\"btn text has-icon st\" data-act=\"copy-path\" data-ref=\"{}\">{}Путь</button>",
        referent.value(),
        icon(s::CONTENT_COPY)
    );
    out.push_str("</div>");
    if children > 0 {
        let _ = write!(
            out,
            "<div class=\"path\" style=\"padding-top:0\">{}</div>",
            plural(
                children,
                "дочерний объект",
                "дочерних объекта",
                "дочерних объектов"
            )
        );
    }
    for group in props::sheet(dom, referent) {
        let _ = write!(
            out,
            "<div class=\"group\"><div class=\"gh\">{}</div><div class=\"gb\">",
            esc(&group.title)
        );
        for property in group.properties {
            let _ = write!(
                out,
                "<div class=\"prop\"><span class=\"k\">{}</span>",
                esc(&property.name)
            );
            match property.value {
                Value::Text(text) => {
                    let _ = write!(out, "<span class=\"v\">{}</span>", esc(&text));
                }
                Value::Mono(text) => {
                    let _ = write!(out, "<span class=\"v mono\">{}</span>", esc(&text));
                }
                Value::Bool(on) => {
                    let _ = write!(
                        out,
                        "<span class=\"v\"><span class=\"bool{}\">{}{}</span></span>",
                        if on { " on" } else { "" },
                        icon(if on { s::CHECK } else { s::CLOSE }),
                        on
                    );
                }
                Value::Color(rgb, text) => {
                    let _ = write!(
                        out,
                        "<span class=\"v\"><span class=\"sw\" style=\"background:rgb({},{},{})\"></span>{}</span>",
                        rgb[0],
                        rgb[1],
                        rgb[2],
                        esc(&text)
                    );
                }
                Value::Ref(target, text) => match target {
                    Some(target) => {
                        let _ = write!(
                            out,
                            "<span class=\"v\"><button class=\"btn text st\" style=\"height:28px;padding:0 8px;margin:-4px -8px\" data-act=\"info\" data-ref=\"{}\">{}</button></span>",
                            target.value(),
                            esc(&text)
                        );
                    }
                    None => {
                        let _ = write!(out, "<span class=\"v\">{}</span>", esc(&text));
                    }
                },
            }
            out.push_str("</div>");
        }
        out.push_str("</div></div>");
    }
    out.push_str("</div></div>");
    out
}

/// The script editor's frame; the lines are drawn by [`code_lines`].
pub fn editor_screen(document: &Document, referent: Ref) -> String {
    let dom = document.dom();
    let name = dom
        .get(referent)
        .map(|instance| instance.name().to_string())
        .unwrap_or_default();
    let mut path = path_names(dom, referent);
    path.pop();
    let mut out = String::new();
    out.push_str(
        "<section class=\"screen\" data-screen=\"editor\"><header class=\"top-bar has-sub\">",
    );
    out.push_str(&icon_btn(
        "close-editor",
        s::ARROW_BACK,
        "Назад",
        " on-surface",
    ));
    let _ = write!(
        out,
        "<div class=\"titles\"><div class=\"title\">{}<span class=\"dirty-dot\" id=\"ed-dirty\"{}></span></div><div class=\"subtitle\">{}</div></div>",
        esc(&name),
        if document.is_edited(referent) { "" } else { " hidden" },
        if path.is_empty() {
            script_tag_text(script_kind(dom, referent)).to_string()
        } else {
            format!(
                "{} · {}",
                script_tag_text(script_kind(dom, referent)),
                esc(&path.join(" › "))
            )
        }
    );
    out.push_str(&icon_btn("undo", s::UNDO, "Отменить", "\" id=\"b-undo"));
    out.push_str(&icon_btn("redo", s::REDO, "Повторить", "\" id=\"b-redo"));
    out.push_str(&icon_btn("find", s::SEARCH, "Найти", "\" id=\"b-find"));
    out.push_str(&icon_btn("editor-menu", s::MORE_VERT, "Ещё", ""));
    out.push_str("</header>");
    let _ = write!(
        out,
        "<div class=\"find\" id=\"find\" hidden><div class=\"fr\"><input id=\"fq\" placeholder=\"Найти\" autocomplete=\"off\" autocapitalize=\"off\" spellcheck=\"false\" enterkeyhint=\"next\"><span class=\"cnt\" id=\"fcnt\"></span>{}{}{}{}</div>",
        icon_btn("find-case", s::MATCH_CASE, "Учитывать регистр", "\" id=\"b-case"),
        icon_btn("find-prev", s::ARROW_UP, "Предыдущее", ""),
        icon_btn("find-next", s::ARROW_DOWN, "Следующее", ""),
        icon_btn("find-close", s::CLOSE, "Закрыть", "")
    );
    out.push_str("<div class=\"fr\"><input id=\"fr\" placeholder=\"Заменить на\" autocomplete=\"off\" autocapitalize=\"off\" spellcheck=\"false\"><button class=\"btn text st\" data-act=\"replace-one\">Заменить</button><button class=\"btn text st\" data-act=\"replace-all\">Все</button></div></div>");
    out.push_str("<div class=\"editor\"><div class=\"code\" id=\"code\"><div class=\"code-inner\" id=\"code-inner\"><div class=\"gutter\" id=\"gutter\"></div><div class=\"text-area\" id=\"text-area\"><div class=\"cur-line\" id=\"cur-line\"></div><div class=\"marks\" id=\"marks\"></div><pre class=\"hl\" id=\"hl\"></pre><textarea class=\"ta\" id=\"ta\" spellcheck=\"false\" autocomplete=\"off\" autocorrect=\"off\" autocapitalize=\"off\" wrap=\"off\" aria-label=\"Код скрипта\"></textarea></div></div></div>");
    out.push_str("<div class=\"keys\" id=\"keys\">");
    let keys: [(&str, String, &str); 3] = [
        ("tab", icon(s::KEYBOARD_TAB), " accent wide"),
        ("left", icon(s::ARROW_LEFT), ""),
        ("right", icon(s::ARROW_RIGHT), ""),
    ];
    for (key, label, class) in keys {
        let _ = write!(
            out,
            "<button class=\"key{class}\" data-act=\"key\" data-key=\"{key}\">{label}</button>"
        );
    }
    for text in [
        "(", ")", "{", "}", "[", "]", "\"", "=", ".", ":", ",", "-", "#", "_", "~", "<", ">", "+",
        "*", "/", "%", "^", "`", "'",
    ] {
        let _ = write!(
            out,
            "<button class=\"key\" data-act=\"key\" data-insert=\"{0}\">{0}</button>",
            esc(text)
        );
    }
    out.push_str("</div></div></section>");
    out
}

fn script_tag_text(kind: Option<ScriptKind>) -> &'static str {
    match kind {
        Some(ScriptKind::Server) => "Script",
        Some(ScriptKind::Client) => "LocalScript",
        Some(ScriptKind::Module) => "ModuleScript",
        None => "",
    }
}

/// The gutter numbers and highlighted lines `range` of the script, given
/// its line starts and the lexer state at each line.
pub fn code_lines(
    text: &str,
    starts: &[usize],
    states: &[State],
    range: std::ops::Range<usize>,
    current: usize,
) -> (String, String) {
    let mut gutter = String::new();
    let mut code = String::new();
    let end = range.end.min(starts.len());
    for line in range.start.min(end)..end {
        let _ = write!(
            gutter,
            "<div class=\"ln{}\">{}</div>",
            if line == current { " cur" } else { "" },
            line + 1
        );
        code.push_str("<div class=\"l\">");
        let body = crate::textops::line_text(text, starts, line);
        let state = states.get(line).copied().unwrap_or_default();
        if body.is_empty() {
            code.push(' ');
        } else {
            render_line(body, state, &mut code);
        }
        code.push_str("</div>");
    }
    (gutter, code)
}

/// A modal dialog. `actions` are `(data-act, label, class)`.
pub fn dialog(
    icon_path: Option<&str>,
    title: &str,
    body: &str,
    actions: &[(&str, &str, &str)],
) -> String {
    let mut out = String::from("<div class=\"scrim\" data-act=\"dismiss\"></div><div class=\"dialog-wrap\"><div class=\"dialog\" role=\"dialog\">");
    if let Some(path) = icon_path {
        let _ = write!(out, "<div class=\"d-icon\">{}</div>", icon(path));
    }
    let _ = write!(
        out,
        "<div class=\"d-title\">{}</div><div class=\"d-text\">{body}</div><div class=\"d-actions\">",
        esc(title)
    );
    for (act, label, class) in actions {
        let _ = write!(
            out,
            "<button class=\"btn {class} st\" data-act=\"{act}\">{}</button>",
            esc(label)
        );
    }
    out.push_str("</div></div></div>");
    out
}

/// A menu item: `(data-act, icon, label, trailing text)`.
pub type MenuItem<'a> = (&'a str, &'a str, &'a str, &'a str);

/// A dropdown menu anchored at the top right, below the app bar.
pub fn menu(items: &[Option<MenuItem<'_>>], top: f64) -> String {
    let mut out = format!(
        "<div class=\"menu-catcher\" data-act=\"dismiss\"></div><div class=\"menu\" style=\"top:{top}px;right:8px\">"
    );
    for item in items {
        match item {
            Some((act, path, label, trailing)) => {
                let _ = write!(
                    out,
                    "<button class=\"mi st\" data-act=\"{act}\">{}<span>{}</span><span class=\"tr\">{}</span></button>",
                    icon(path),
                    esc(label),
                    esc(trailing)
                );
            }
            None => out.push_str("<div class=\"sep\"></div>"),
        }
    }
    out.push_str("</div>");
    out
}

/// A snackbar, optionally with an action button.
pub fn snackbar(message: &str, action: Option<(&str, &str)>) -> String {
    let mut out = format!(
        "<div class=\"snackbar\" role=\"status\"><span class=\"msg\">{}</span>",
        esc(message)
    );
    if let Some((act, label)) = action {
        let _ = write!(
            out,
            "<button class=\"btn text st\" data-act=\"{act}\">{}</button>",
            esc(label)
        );
    }
    out.push_str("</div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Document {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/tests/FPS.rbxm"
        ))
        .unwrap();
        Document::open("FPS.rbxm", &bytes).unwrap()
    }

    #[test]
    fn screens_render() {
        let document = document();
        let explorer = Explorer::new(document.dom());
        let html = document_screen(&document, Tab::Tree, "", 3);
        assert!(html.contains("FPS.rbxm"));
        let rows = tree_rows(&document, &explorer, 0..50, None);
        assert!(rows.contains("class=\"node st"));
        let scripts = crate::explorer::all_scripts(document.dom());
        assert!(!scripts.is_empty());
        let list = script_list(&document, &scripts, "");
        assert!(list.contains("open-script"));
        let first = scripts[0];
        assert!(sheet(&document, first).contains("Открыть скрипт"));
        assert!(editor_screen(&document, first).contains("id=\"ta\""));
        let text = document.source(first).unwrap();
        let starts = crate::textops::line_starts(&text);
        let states = crate::highlight::line_states(&text);
        let (gutter, code) = code_lines(&text, &starts, &states, 0..10, 0);
        assert!(gutter.contains("ln cur"));
        assert_eq!(code.matches("class=\"l\"").count(), 10.min(starts.len()));
    }

    #[test]
    fn names_are_escaped_and_marked() {
        assert_eq!(
            marked_name("<Kill>Brick", "kill"),
            "&lt;<mark>Kill</mark>&gt;Brick"
        );
        assert_eq!(esc("a\"b&"), "a&quot;b&amp;");
    }
}
