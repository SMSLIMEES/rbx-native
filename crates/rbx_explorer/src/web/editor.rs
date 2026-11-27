//! The script editor: a transparent `<textarea>` the user types into, laid
//! exactly over a `<pre>` holding the highlighted lines in view. The
//! textarea is as large as the whole script, so the surrounding `.code`
//! box is what scrolls, and only the lines on screen are ever turned into
//! HTML, so a ten-thousand-line script scrolls as freely as a short one.

use rbx_dom::Ref;

use super::dom;
use crate::highlight::{line_states, State};
use crate::history::{Edit, History};
use crate::textops::{
    byte_of, find_all, indent_after, indent_of, line_of, line_starts, line_text, max_columns,
    outdented, replace_all, utf16_of, visual_width,
};
use crate::ui;

/// Space above the first line and left of every line, as `.ta` pads it.
const PAD_TOP: f64 = 8.0;
const PAD_LEFT: f64 = 12.0;
/// Lines drawn beyond the visible ones, so a fling shows no blank.
const OVERSCAN: usize = 20;

pub struct Editor {
    pub referent: Ref,
    pub text: String,
    starts: Vec<usize>,
    states: Vec<State>,
    columns: usize,
    pub history: History,
    /// Selection in bytes, as last seen.
    selection: (usize, usize),
    char_width: f64,
    line_height: f64,
    drawn: (usize, usize),
    caret_line: usize,
    pub find_open: bool,
    pub case_sensitive: bool,
    query: String,
    matches: Vec<(usize, usize)>,
    current: Option<usize>,
}

impl Editor {
    pub fn new(referent: Ref, text: String, history: History) -> Editor {
        let mut editor = Editor {
            referent,
            text,
            starts: Vec::new(),
            states: Vec::new(),
            columns: 0,
            history,
            selection: (0, 0),
            char_width: 7.8,
            line_height: 20.0,
            drawn: (usize::MAX, 0),
            caret_line: 0,
            find_open: false,
            case_sensitive: false,
            query: String::new(),
            matches: Vec::new(),
            current: None,
        };
        editor.reindex();
        editor
    }

    fn reindex(&mut self) {
        self.starts = line_starts(&self.text);
        self.states = line_states(&self.text);
        self.columns = max_columns(&self.text);
    }

    /// After the screen's HTML is in place: fill the textarea, measure,
    /// size and draw.
    pub fn mount(&mut self, font_size: f64) {
        if let Some(textarea) = dom::textarea("ta") {
            textarea.set_value(&self.text);
            let _ = textarea.set_selection_range(0, 0);
        }
        self.apply_font(font_size);
    }

    /// Applies a code font size, remeasures and redraws.
    pub fn apply_font(&mut self, font_size: f64) {
        let line_height = (font_size * 1.54).round();
        if let Some(code) = dom::by_id("code") {
            dom::set_style(&code, "--code-size", &format!("{font_size}px"));
            dom::set_style(&code, "--code-line-height", &format!("{line_height}px"));
        }
        self.line_height = line_height;
        self.char_width = measure_char_width().unwrap_or(font_size * 0.6);
        self.layout();
        self.drawn = (usize::MAX, 0);
        self.draw(true);
    }

    /// Sizes the scrolling content to the whole script.
    fn layout(&self) {
        let lines = self.starts.len() as f64;
        let height = lines * self.line_height + PAD_TOP * 2.0;
        let digits = self.starts.len().to_string().len().max(2) as f64;
        let gutter = digits * self.char_width * 0.95 + 20.0;
        dom::set_style_id("code-inner", "height", &format!("{height}px"));
        dom::set_style_id("gutter", "min-width", &format!("{gutter}px"));
        let mut width = (self.columns as f64 + 3.0) * self.char_width + PAD_LEFT + 16.0;
        if let Some(textarea) = dom::textarea("ta") {
            let scroll = f64::from(textarea.scroll_width());
            let client = f64::from(textarea.client_width());
            if scroll > client + 1.0 {
                width = width.max(scroll + self.char_width * 4.0);
            }
        }
        dom::set_style_id("text-area", "min-width", &format!("{width}px"));
    }

    fn viewport(&self) -> (f64, f64, f64, f64) {
        match dom::html_by_id("code") {
            Some(code) => (
                f64::from(code.scroll_top()),
                f64::from(code.client_height()),
                f64::from(code.scroll_left()),
                f64::from(code.client_width()),
            ),
            None => (0.0, 600.0, 0.0, 400.0),
        }
    }

    fn gutter_width(&self) -> f64 {
        dom::html_by_id("gutter").map_or(44.0, |gutter| f64::from(gutter.offset_width()))
    }

    /// Draws the lines in view (if they changed, or always with `force`).
    pub fn draw(&mut self, force: bool) {
        let (top, height, _, _) = self.viewport();
        let first = ((top - PAD_TOP) / self.line_height).floor().max(0.0) as usize;
        let last = ((top + height) / self.line_height).ceil() as usize;
        let start = first.saturating_sub(OVERSCAN);
        let end = (last + OVERSCAN).min(self.starts.len());
        if !force && start >= self.drawn.0 && end <= self.drawn.1 && self.drawn.0 != usize::MAX {
            return;
        }
        // Draw a bit more than needed so small scrolls redraw nothing.
        let start = start.saturating_sub(OVERSCAN);
        let end = (end + OVERSCAN).min(self.starts.len());
        self.drawn = (start, end);
        let (gutter, code) = ui::code_lines(
            &self.text,
            &self.starts,
            &self.states,
            start..end,
            self.caret_line,
        );
        let offset = PAD_TOP + start as f64 * self.line_height;
        dom::set_html("gutter", &gutter);
        dom::set_style_id("gutter", "padding-top", &format!("{offset}px"));
        dom::set_html("hl", &code);
        dom::set_style_id("hl", "top", &format!("{offset}px"));
        self.draw_marks();
        self.draw_caret_line();
    }

    fn draw_caret_line(&self) {
        let top = PAD_TOP + self.caret_line as f64 * self.line_height;
        dom::set_style_id("cur-line", "top", &format!("{top}px"));
    }

    fn draw_marks(&self) {
        let mut html = String::new();
        if self.find_open && !self.matches.is_empty() {
            let (start, end) = self.drawn;
            let from = self.starts.get(start).copied().unwrap_or(0);
            let first = self.matches.partition_point(|(at, _)| *at < from);
            for (index, (at, to)) in self.matches.iter().enumerate().skip(first) {
                let line = line_of(&self.starts, *at);
                if line >= end {
                    break;
                }
                let line_start = self.starts[line];
                let body = line_text(&self.text, &self.starts, line);
                let col = visual_width(&self.text[line_start..*at], 0);
                let stop = (*to).min(line_start + body.len());
                let width = visual_width(&self.text[*at..stop], col).max(1);
                html.push_str(&format!(
                    "<div class=\"mk{}\" style=\"left:{}px;top:{}px;width:{}px\"></div>",
                    if self.current == Some(index) {
                        " cur"
                    } else {
                        ""
                    },
                    PAD_LEFT + col as f64 * self.char_width,
                    PAD_TOP + line as f64 * self.line_height,
                    width as f64 * self.char_width
                ));
            }
        }
        dom::set_html("marks", &html);
    }

    /// Reads the textarea's selection into `selection`; returns whether
    /// the caret moved to another line.
    pub fn sync_selection(&mut self) -> bool {
        let Some(textarea) = dom::textarea("ta") else {
            return false;
        };
        let start = textarea.selection_start().ok().flatten().unwrap_or(0) as usize;
        let end = textarea.selection_end().ok().flatten().unwrap_or(0) as usize;
        self.selection = (byte_of(&self.text, start), byte_of(&self.text, end));
        let line = line_of(&self.starts, self.selection.1);
        if line != self.caret_line {
            self.caret_line = line;
            self.draw_caret_line();
            self.redraw_gutter_current();
            return true;
        }
        false
    }

    fn redraw_gutter_current(&self) {
        let (start, end) = self.drawn;
        if start == usize::MAX {
            return;
        }
        // Only the gutter's `cur` mark changes: rebuild just the numbers.
        let mut numbers = String::new();
        for line in start..end.min(self.starts.len()) {
            numbers.push_str(&format!(
                "<div class=\"ln{}\">{}</div>",
                if line == self.caret_line { " cur" } else { "" },
                line + 1
            ));
        }
        dom::set_html("gutter", &numbers);
    }

    /// The user typed (or pasted, or cut) into the textarea: take its
    /// text, record the edit, redraw. Returns whether the text changed.
    pub fn on_input(&mut self) -> bool {
        let Some(textarea) = dom::textarea("ta") else {
            return false;
        };
        let new = textarea.value();
        let before = self.selection;
        let start = textarea.selection_start().ok().flatten().unwrap_or(0) as usize;
        let end = textarea.selection_end().ok().flatten().unwrap_or(0) as usize;
        let after = (byte_of(&new, start), byte_of(&new, end));
        let Some(edit) = Edit::between(&self.text, &new, before, after) else {
            return false;
        };
        self.history.record(edit, dom::now_ms());
        self.text = new;
        self.selection = after;
        self.changed();
        true
    }

    /// Replaces bytes `start..end` with `insert` and puts the selection at
    /// `selection` (bytes in the new text), as one undoable step.
    pub fn replace(&mut self, start: usize, end: usize, insert: &str, selection: (usize, usize)) {
        let old = self.text.clone();
        let mut new = String::with_capacity(old.len() + insert.len());
        new.push_str(&old[..start]);
        new.push_str(insert);
        new.push_str(&old[end..]);
        if let Some(edit) = Edit::between(&old, &new, self.selection, selection) {
            // Never merged with typing: a key-bar press is its own step.
            self.history.record(edit, f64::NEG_INFINITY);
        }
        self.set_text(new, selection);
    }

    fn set_text(&mut self, text: String, selection: (usize, usize)) {
        self.text = text;
        if let Some(textarea) = dom::textarea("ta") {
            textarea.set_value(&self.text);
            let _ = textarea.set_selection_range(
                utf16_of(&self.text, selection.0) as u32,
                utf16_of(&self.text, selection.1) as u32,
            );
        }
        self.selection = selection;
        self.changed();
        self.reveal_caret();
    }

    fn changed(&mut self) {
        let lines = self.starts.len();
        let columns = self.columns;
        self.reindex();
        if lines != self.starts.len() || columns != self.columns {
            self.layout();
        }
        self.caret_line = line_of(&self.starts, self.selection.1);
        if self.find_open {
            self.refind(false);
        }
        self.draw(true);
    }

    pub fn undo(&mut self) -> bool {
        let mut text = self.text.clone();
        match self.history.undo(&mut text) {
            Some(selection) => {
                self.set_text(text, selection);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        let mut text = self.text.clone();
        match self.history.redo(&mut text) {
            Some(selection) => {
                self.set_text(text, selection);
                true
            }
            None => false,
        }
    }

    /// Types `text` over the selection.
    pub fn insert(&mut self, text: &str) {
        self.sync_selection();
        let (start, end) = self.selection;
        let caret = start + text.len();
        self.replace(start, end, text, (caret, caret));
    }

    /// Enter: a line break carrying the line's indentation, one level
    /// deeper after a block opener.
    pub fn newline(&mut self) {
        self.sync_selection();
        let (start, _) = self.selection;
        let line = line_of(&self.starts, start);
        let line_start = self.starts[line];
        let indent = indent_after(&self.text[line_start..start]);
        self.insert(&format!("\n{indent}"));
    }

    /// Tab: a tab at the caret, or with lines selected, indents them all
    /// (`outdent` for Shift+Tab).
    pub fn tab(&mut self, outdent: bool) {
        self.sync_selection();
        let (start, end) = self.selection;
        let first = line_of(&self.starts, start);
        let last = line_of(&self.starts, end.saturating_sub(usize::from(end > start)));
        if first == last && !outdent {
            self.insert("\t");
            return;
        }
        let block_start = self.starts[first];
        let block_end = self
            .starts
            .get(last + 1)
            .map_or(self.text.len(), |next| next - 1);
        let mut out = String::new();
        let mut first_shift: isize = 0;
        for (index, line) in self.text[block_start..block_end].split('\n').enumerate() {
            if index > 0 {
                out.push('\n');
            }
            let indent = indent_of(line);
            let new_indent = if outdent {
                outdented(indent).to_string()
            } else {
                format!("\t{indent}")
            };
            if index == 0 {
                first_shift = new_indent.len() as isize - indent.len() as isize;
            }
            out.push_str(&new_indent);
            out.push_str(&line[indent.len()..]);
        }
        let new_end = block_start + out.len();
        let new_start = (start as isize + first_shift).max(block_start as isize) as usize;
        let selection = if start == end {
            (new_start, new_start)
        } else {
            (new_start, new_end)
        };
        self.replace(block_start, block_end, &out, selection);
    }

    /// Moves the caret one character left or right.
    pub fn step(&mut self, forward: bool) {
        self.sync_selection();
        let (start, end) = self.selection;
        let caret = if start != end {
            if forward {
                end
            } else {
                start
            }
        } else if forward {
            self.text[end..]
                .chars()
                .next()
                .map_or(end, |ch| end + ch.len_utf8())
        } else {
            self.text[..start]
                .chars()
                .next_back()
                .map_or(start, |ch| start - ch.len_utf8())
        };
        self.select(caret, caret);
    }

    fn select(&mut self, start: usize, end: usize) {
        if let Some(textarea) = dom::textarea("ta") {
            let _ = textarea.set_selection_range(
                utf16_of(&self.text, start) as u32,
                utf16_of(&self.text, end) as u32,
            );
        }
        self.selection = (start, end);
        self.caret_line = line_of(&self.starts, end);
        self.draw_caret_line();
        self.redraw_gutter_current();
        self.reveal_caret();
    }

    /// Scrolls so the caret (the selection's end) is in view.
    pub fn reveal_caret(&self) {
        self.reveal(self.selection.1);
    }

    fn reveal(&self, offset: usize) {
        let Some(code) = dom::html_by_id("code") else {
            return;
        };
        let (top, height, left, width) = self.viewport();
        let line = line_of(&self.starts, offset);
        let line_start = self.starts[line];
        let column = visual_width(&self.text[line_start..offset.min(self.text.len())], 0);
        let y = PAD_TOP + line as f64 * self.line_height;
        let margin = self.line_height * 2.0;
        if y < top + margin {
            code.set_scroll_top((y - margin).max(0.0) as i32);
        } else if y + self.line_height > top + height - margin {
            code.set_scroll_top((y + self.line_height + margin - height) as i32);
        }
        let gutter = self.gutter_width();
        let x = PAD_LEFT + column as f64 * self.char_width;
        let visible = width - gutter;
        if x < left + self.char_width {
            code.set_scroll_left((x - self.char_width * 6.0).max(0.0) as i32);
        } else if x > left + visible - self.char_width * 2.0 {
            code.set_scroll_left((x - visible + self.char_width * 4.0) as i32);
        }
    }

    /// Scrolls to line `line` (1-based) and puts the caret there.
    pub fn go_to_line(&mut self, line: usize) {
        let index = line.saturating_sub(1).min(self.starts.len() - 1);
        let at = self.starts[index];
        if let Some(code) = dom::html_by_id("code") {
            let (_, height, _, _) = self.viewport();
            let y = PAD_TOP + index as f64 * self.line_height;
            code.set_scroll_top((y - height / 3.0).max(0.0) as i32);
        }
        self.select(at, at);
        self.draw(false);
    }

    pub fn line_count(&self) -> usize {
        self.starts.len()
    }

    // ---- find & replace ------------------------------------------------

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.refind(true);
        self.draw_marks();
        self.show_count();
    }

    /// Recomputes the matches; `jump` moves to the first one at or after
    /// the caret.
    fn refind(&mut self, jump: bool) {
        self.matches = find_all(&self.text, &self.query, self.case_sensitive);
        if self.matches.is_empty() {
            self.current = None;
        } else if jump || self.current.is_none() {
            let from = self.selection.0;
            let index = self.matches.partition_point(|(at, _)| *at < from);
            self.current = Some(index % self.matches.len());
            if jump {
                self.show_current();
            }
        } else if let Some(current) = self.current {
            self.current = Some(current.min(self.matches.len() - 1));
        }
        self.show_count();
    }

    fn show_count(&self) {
        let text = match (self.query.is_empty(), self.current) {
            (true, _) => String::new(),
            (false, None) => "0 / 0".to_string(),
            (false, Some(index)) => format!("{} / {}", index + 1, self.matches.len()),
        };
        dom::set_html("fcnt", &text);
    }

    fn show_current(&mut self) {
        let Some(index) = self.current else {
            return;
        };
        let (start, end) = self.matches[index];
        if let Some(textarea) = dom::textarea("ta") {
            let _ = textarea.set_selection_range(
                utf16_of(&self.text, start) as u32,
                utf16_of(&self.text, end) as u32,
            );
        }
        self.selection = (start, end);
        self.caret_line = line_of(&self.starts, start);
        self.reveal(end);
        self.reveal(start);
        self.draw(false);
        self.draw_caret_line();
        self.redraw_gutter_current();
        self.draw_marks();
    }

    pub fn find_step(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        let count = self.matches.len();
        self.current = Some(match self.current {
            None => 0,
            Some(index) if forward => (index + 1) % count,
            Some(index) => (index + count - 1) % count,
        });
        self.show_count();
        self.show_current();
    }

    pub fn toggle_case(&mut self) {
        self.case_sensitive = !self.case_sensitive;
        self.refind(true);
        self.draw_marks();
    }

    pub fn open_find(&mut self) {
        self.find_open = true;
        if self.selection.0 < self.selection.1 {
            let selected = &self.text[self.selection.0..self.selection.1];
            if !selected.contains('\n') && selected.len() < 200 {
                self.query = selected.to_string();
                if let Some(input) = dom::input("fq") {
                    input.set_value(&self.query);
                }
            }
        }
        self.refind(false);
        self.draw_marks();
    }

    pub fn close_find(&mut self) {
        self.find_open = false;
        self.matches.clear();
        self.current = None;
        self.draw_marks();
    }

    /// Replaces the current match; returns whether one was replaced.
    pub fn replace_one(&mut self, with: &str) -> bool {
        let Some(index) = self.current else {
            return false;
        };
        let (start, end) = self.matches[index];
        let caret = start + with.len();
        self.replace(start, end, with, (caret, caret));
        // `changed` refound; move on to the next match after the caret.
        self.refind(true);
        self.draw_marks();
        true
    }

    /// Replaces every match; returns how many.
    pub fn replace_every(&mut self, with: &str) -> usize {
        let count = self.matches.len();
        if count == 0 {
            return 0;
        }
        let new = replace_all(&self.text, &self.query, with, self.case_sensitive);
        let length = self.text.len();
        let caret = self.selection.0.min(new.len());
        let mut caret = caret;
        while !new.is_char_boundary(caret) {
            caret -= 1;
        }
        self.replace(0, length, &new, (caret, caret));
        count
    }
}

/// The advance of one character of the code font, measured on the page.
fn measure_char_width() -> Option<f64> {
    let document = dom::document();
    let probe = document.create_element("span").ok()?;
    probe.set_class_name("hl");
    let _ = probe.set_attribute(
        "style",
        "position:absolute;visibility:hidden;padding:0;left:0;top:0;width:auto;",
    );
    probe.set_text_content(Some(&"M".repeat(200)));
    let parent = dom::by_id("text-area")?;
    parent.append_child(&probe).ok()?;
    let width = probe.get_bounding_client_rect().width() / 200.0;
    let _ = parent.remove_child(&probe);
    (width > 0.0).then_some(width)
}
