//! Text arithmetic the editor needs: line tables, UTF-16 offsets (what a
//! `<textarea>`'s selection is measured in), visual columns with tabs, search
//! and auto-indentation.

/// Width of a tab stop, matching the editor's CSS `tab-size`.
pub const TAB_WIDTH: usize = 4;

/// Byte offset of the start of every line.
pub fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(text.len() / 32 + 1);
    starts.push(0);
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(index + 1);
        }
    }
    starts
}

/// The line holding byte `offset`.
pub fn line_of(starts: &[usize], offset: usize) -> usize {
    match starts.binary_search(&offset) {
        Ok(line) => line,
        Err(next) => next.saturating_sub(1),
    }
}

/// The text of line `index` (without its `\n`).
pub fn line_text<'a>(text: &'a str, starts: &[usize], index: usize) -> &'a str {
    let Some(&start) = starts.get(index) else {
        return "";
    };
    let end = starts
        .get(index + 1)
        .map_or(text.len(), |next| next.saturating_sub(1));
    &text[start..end.max(start)]
}

/// How many columns `segment` takes when it starts at column `from`, with
/// tabs advancing to the next stop.
pub fn visual_width(segment: &str, from: usize) -> usize {
    let mut column = from;
    for ch in segment.chars() {
        if ch == '\t' {
            column += TAB_WIDTH - column % TAB_WIDTH;
        } else {
            column += 1;
        }
    }
    column - from
}

/// The widest line, in columns.
pub fn max_columns(text: &str) -> usize {
    text.split('\n')
        .map(|line| visual_width(line, 0))
        .max()
        .unwrap_or(0)
}

/// The UTF-16 offset of byte offset `byte` (clamped to a char boundary).
pub fn utf16_of(text: &str, byte: usize) -> usize {
    let mut byte = byte.min(text.len());
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    if text.is_ascii() {
        return byte;
    }
    text[..byte].encode_utf16().count()
}

/// The byte offset of UTF-16 offset `units`.
pub fn byte_of(text: &str, units: usize) -> usize {
    if text.is_ascii() {
        return units.min(text.len());
    }
    let mut seen = 0;
    for (index, ch) in text.char_indices() {
        if seen >= units {
            return index;
        }
        seen += ch.len_utf16();
    }
    text.len()
}

/// Every match of `needle` in `haystack` as byte ranges, ASCII case
/// insensitive (which keeps byte offsets intact, unlike full Unicode case
/// folding). An empty needle matches nothing.
pub fn find_all(haystack: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return Vec::new();
    }
    let hay = haystack.as_bytes();
    let pin = needle.as_bytes();
    let eq = |a: u8, b: u8| {
        if case_sensitive {
            a == b
        } else {
            a.eq_ignore_ascii_case(&b)
        }
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i + pin.len() <= hay.len() {
        if eq(hay[i], pin[0])
            && hay[i..i + pin.len()]
                .iter()
                .zip(pin)
                .all(|(a, b)| eq(*a, *b))
            && haystack.is_char_boundary(i)
            && haystack.is_char_boundary(i + pin.len())
        {
            found.push((i, i + pin.len()));
            i += pin.len();
        } else {
            i += 1;
        }
    }
    found
}

/// `haystack` with every match of `needle` replaced by `with`.
pub fn replace_all(haystack: &str, needle: &str, with: &str, case_sensitive: bool) -> String {
    let matches = find_all(haystack, needle, case_sensitive);
    let mut out = String::with_capacity(haystack.len());
    let mut last = 0;
    for (start, end) in matches {
        out.push_str(&haystack[last..start]);
        out.push_str(with);
        last = end;
    }
    out.push_str(&haystack[last..]);
    out
}

/// The leading whitespace of `line`.
pub fn indent_of(line: &str) -> &str {
    let end = line
        .find(|ch: char| ch != ' ' && ch != '\t')
        .unwrap_or(line.len());
    &line[..end]
}

/// Whether the line after `line` should be indented one level deeper:
/// it opens a block (`then`, `do`, `function(...)`, `repeat`, `else`) or a
/// bracket it does not close.
pub fn opens_block(line: &str) -> bool {
    // Comments do not open anything.
    let code = match line.find("--") {
        Some(at) => &line[..at],
        None => line,
    };
    let trimmed = code.trim_end();
    let last_word = trimmed
        .rsplit(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .next()
        .unwrap_or("");
    if matches!(last_word, "then" | "do" | "repeat" | "else") {
        return true;
    }
    if trimmed.ends_with('{') || trimmed.ends_with('(') || trimmed.ends_with('[') {
        return true;
    }
    // `function name(args)` / `= function(args)` without a closing `end`.
    let words: Vec<&str> = trimmed
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let functions = words.iter().filter(|word| **word == "function").count();
    let ends = words.iter().filter(|word| **word == "end").count();
    functions > ends && trimmed.ends_with(')')
}

/// What to type after a line break so the new line lines up: the previous
/// line's indentation, one level deeper after a block opener.
pub fn indent_after(previous_line: &str) -> String {
    let mut indent = indent_of(previous_line).to_string();
    if opens_block(previous_line) {
        // Follow the file's own style: spaces if it indents with spaces.
        if indent.starts_with(' ') {
            indent.push_str(&" ".repeat(TAB_WIDTH));
        } else {
            indent.push('\t');
        }
    }
    indent
}

/// `line`'s leading whitespace with one level removed (a tab, or up to
/// [`TAB_WIDTH`] spaces).
pub fn outdented(indent: &str) -> &str {
    if let Some(rest) = indent.strip_suffix('\t') {
        return rest;
    }
    let spaces = indent.len() - indent.trim_end_matches(' ').len();
    &indent[..indent.len() - spaces.min(TAB_WIDTH)]
}

/// Number of lines in `text`.
pub fn line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_offsets() {
        let text = "ab\ncd\n\nlast";
        let starts = line_starts(text);
        assert_eq!(starts, vec![0, 3, 6, 7]);
        assert_eq!(line_of(&starts, 0), 0);
        assert_eq!(line_of(&starts, 4), 1);
        assert_eq!(line_of(&starts, 6), 2);
        assert_eq!(line_of(&starts, 10), 3);
        assert_eq!(line_text(text, &starts, 1), "cd");
        assert_eq!(line_text(text, &starts, 2), "");
        assert_eq!(line_text(text, &starts, 3), "last");
        assert_eq!(line_count(text), 4);
    }

    #[test]
    fn utf16_round_trips() {
        let text = "a—b😀c";
        for (byte, units) in [(0, 0), (1, 1), (4, 2), (5, 3), (9, 5), (10, 6)] {
            assert_eq!(utf16_of(text, byte), units);
            assert_eq!(byte_of(text, units), byte);
        }
    }

    #[test]
    fn tabs_advance_to_stops() {
        assert_eq!(visual_width("\tx", 0), 5);
        assert_eq!(visual_width("ab\tx", 0), 5);
        assert_eq!(max_columns("a\n\t\tb\nccc"), 9);
    }

    #[test]
    fn search_and_replace() {
        let text = "Part part PART pärt";
        assert_eq!(find_all(text, "part", false).len(), 3);
        assert_eq!(find_all(text, "part", true), vec![(5, 9)]);
        assert_eq!(replace_all(text, "part", "x", false), "x x x pärt");
        assert!(find_all(text, "", false).is_empty());
    }

    #[test]
    fn indentation() {
        assert_eq!(indent_after("\tif x then"), "\t\t");
        assert_eq!(indent_after("    for i = 1, 3 do"), "        ");
        assert_eq!(indent_after("local function f(a, b)"), "\t");
        assert_eq!(indent_after("local t = {"), "\t");
        assert_eq!(indent_after("\tprint(1)"), "\t");
        assert_eq!(indent_after("x = function() end"), "");
        assert_eq!(indent_after("y = 1 -- then"), "");
        assert_eq!(outdented("\t\t"), "\t");
        assert_eq!(outdented("      "), "  ");
        assert_eq!(outdented(""), "");
    }
}
