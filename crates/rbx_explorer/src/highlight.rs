//! A line-at-a-time Luau highlighter.
//!
//! The editor only ever draws the lines on screen, so the lexer works one line
//! at a time and carries the only state that crosses a line break, being
//! inside a long string `[[ ]]` or a long comment `--[[ ]]`, in [`State`].
//! [`line_states`] runs it over a whole script once per edit (cheap: no HTML
//! is produced), and [`render_line`] turns just the visible lines into spans.

/// What the lexer is inside of at the start of a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Code,
    /// Inside `[==[ ... ]==]`, with that many `=`.
    LongString(u16),
    /// Inside `--[==[ ... ]==]`.
    LongComment(u16),
}

/// What a piece of a line is, which picks its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Text,
    Keyword,
    String,
    Number,
    Comment,
    Builtin,
    Function,
    Constant,
    SelfRef,
    Operator,
    Type,
}

impl Class {
    /// The CSS class of the span, `None` for plain text.
    pub fn css(self) -> Option<&'static str> {
        match self {
            Class::Text => None,
            Class::Keyword => Some("s-kw"),
            Class::String => Some("s-str"),
            Class::Number => Some("s-num"),
            Class::Comment => Some("s-com"),
            Class::Builtin => Some("s-bi"),
            Class::Function => Some("s-fn"),
            Class::Constant => Some("s-const"),
            Class::SelfRef => Some("s-self"),
            Class::Operator => Some("s-op"),
            Class::Type => Some("s-ty"),
        }
    }
}

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "for", "function", "if",
    "in", "local", "not", "or", "repeat", "return", "then", "until", "while",
];

const BUILTINS: &[&str] = &[
    "_G",
    "_VERSION",
    "assert",
    "bit32",
    "BrickColor",
    "buffer",
    "CFrame",
    "Color3",
    "ColorSequence",
    "ColorSequenceKeypoint",
    "coroutine",
    "DateTime",
    "debug",
    "delay",
    "DockWidgetPluginGuiInfo",
    "elapsedTime",
    "Enum",
    "error",
    "Faces",
    "Font",
    "game",
    "gcinfo",
    "getfenv",
    "getmetatable",
    "Instance",
    "ipairs",
    "math",
    "newproxy",
    "next",
    "NumberRange",
    "NumberSequence",
    "NumberSequenceKeypoint",
    "OverlapParams",
    "pairs",
    "PathWaypoint",
    "pcall",
    "PhysicalProperties",
    "plugin",
    "print",
    "Random",
    "rawequal",
    "rawget",
    "rawlen",
    "rawset",
    "Ray",
    "RaycastParams",
    "Rect",
    "Region3",
    "Region3int16",
    "require",
    "script",
    "select",
    "setfenv",
    "setmetatable",
    "shared",
    "spawn",
    "string",
    "table",
    "task",
    "tick",
    "time",
    "tonumber",
    "tostring",
    "TweenInfo",
    "type",
    "typeof",
    "UDim",
    "UDim2",
    "unpack",
    "utf8",
    "Vector2",
    "Vector2int16",
    "Vector3",
    "Vector3int16",
    "vector",
    "wait",
    "warn",
    "workspace",
    "xpcall",
];

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_ident(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `[`, any number of `=`, `[`, starting at `at`: the level and the length
/// of the opener.
fn long_bracket(bytes: &[u8], at: usize) -> Option<(u16, usize)> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut i = at + 1;
    while bytes.get(i) == Some(&b'=') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'[') {
        let level = u16::try_from(i - at - 1).unwrap_or(u16::MAX);
        Some((level, i + 1 - at))
    } else {
        None
    }
}

/// The end (exclusive) of the `]==]` closing a long bracket of `level`, if
/// this line has one at or after `from`.
fn long_close(bytes: &[u8], from: usize, level: u16) -> Option<usize> {
    let level = usize::from(level);
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b']' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b'=' {
                j += 1;
            }
            if j - i - 1 == level && bytes.get(j) == Some(&b']') {
                return Some(j + 1);
            }
        }
        i += 1;
    }
    None
}

/// The next byte at or after `from` that is not a space or tab.
fn next_significant(bytes: &[u8], from: usize) -> Option<u8> {
    bytes[from.min(bytes.len())..]
        .iter()
        .copied()
        .find(|b| *b != b' ' && *b != b'\t')
}

/// Splits `line` into classified pieces, calling `emit` for each in order
/// (their concatenation is exactly `line`), and returns the state the next
/// line starts in.
pub fn lex_line(line: &str, state: State, emit: &mut dyn FnMut(Class, &str)) -> State {
    let bytes = line.as_bytes();
    let mut i = 0;

    match state {
        State::Code => {}
        State::LongString(level) | State::LongComment(level) => {
            let class = if matches!(state, State::LongString(_)) {
                Class::String
            } else {
                Class::Comment
            };
            match long_close(bytes, 0, level) {
                Some(end) => {
                    emit(class, &line[..end]);
                    i = end;
                }
                None => {
                    if !line.is_empty() {
                        emit(class, line);
                    }
                    return state;
                }
            }
        }
    }

    // The last significant byte before the current token, and whether
    // whitespace separated the two: `a:b` is a method, `x: T` a type.
    let mut prev: Option<u8> = None;
    let mut spaced = false;
    // Just after `function`, every name up to the `(` is the function's.
    let mut naming_function = false;

    while i < bytes.len() {
        let byte = bytes[i];
        let start = i;

        if byte == b' ' || byte == b'\t' || byte == b'\r' {
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\r') {
                i += 1;
            }
            emit(Class::Text, &line[start..i]);
            spaced = true;
            continue;
        }

        // Comments.
        if byte == b'-' && bytes.get(i + 1) == Some(&b'-') {
            if let Some((level, len)) = long_bracket(bytes, i + 2) {
                match long_close(bytes, i + 2 + len, level) {
                    Some(end) => {
                        emit(Class::Comment, &line[start..end]);
                        i = end;
                        spaced = true;
                        continue;
                    }
                    None => {
                        emit(Class::Comment, &line[start..]);
                        return State::LongComment(level);
                    }
                }
            }
            emit(Class::Comment, &line[start..]);
            return State::Code;
        }

        // Long strings.
        if byte == b'[' {
            if let Some((level, len)) = long_bracket(bytes, i) {
                match long_close(bytes, i + len, level) {
                    Some(end) => {
                        emit(Class::String, &line[start..end]);
                        i = end;
                        prev = Some(b']');
                        spaced = false;
                        continue;
                    }
                    None => {
                        emit(Class::String, &line[start..]);
                        return State::LongString(level);
                    }
                }
            }
        }

        // Quoted strings.
        if byte == b'"' || byte == b'\'' {
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 2,
                    b if b == byte => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            let end = i.min(bytes.len());
            emit(Class::String, &line[start..end]);
            i = end;
            prev = Some(byte);
            spaced = false;
            continue;
        }

        // Interpolated strings: the `{expressions}` inside are code.
        if byte == b'`' {
            i = lex_interpolated(line, i, emit);
            prev = Some(b'`');
            spaced = false;
            continue;
        }

        // Numbers.
        if byte.is_ascii_digit()
            || (byte == b'.'
                && prev != Some(b'.')
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            i = number_end(bytes, i);
            emit(Class::Number, &line[start..i]);
            prev = Some(b'0');
            spaced = false;
            continue;
        }

        // Names.
        if is_ident_start(byte) {
            while i < bytes.len() && is_ident(bytes[i]) {
                i += 1;
            }
            let word = &line[start..i];
            let after = next_significant(bytes, i);
            let member = matches!(prev, Some(b'.')) || (prev == Some(b':') && !spaced);
            let class = if KEYWORDS.contains(&word) && !member {
                Class::Keyword
            } else if matches!(word, "true" | "false" | "nil") && !member {
                Class::Constant
            } else if word == "self" && !member {
                Class::SelfRef
            } else if naming_function {
                Class::Function
            } else if prev == Some(b':') && spaced {
                Class::Type
            } else if matches!(after, Some(b'(') | Some(b'"') | Some(b'\'') | Some(b'{'))
                && (member || !BUILTINS.contains(&word))
            {
                Class::Function
            } else if !member && BUILTINS.contains(&word) {
                Class::Builtin
            } else {
                Class::Text
            };
            emit(class, word);
            if word == "function" && !member {
                naming_function = true;
            } else if naming_function && !matches!(after, Some(b'.') | Some(b':')) {
                naming_function = false;
            }
            prev = Some(b'a');
            spaced = false;
            continue;
        }

        // Anything else that is ASCII is punctuation; a run of other bytes
        // (letters of other alphabets, emoji) is text up to the next ASCII.
        if byte.is_ascii() {
            i += 1;
            emit(Class::Operator, &line[start..i]);
            if byte == b'(' {
                naming_function = false;
            }
            prev = Some(byte);
            spaced = false;
        } else {
            while i < bytes.len() && !bytes[i].is_ascii() {
                i += 1;
            }
            emit(Class::Text, &line[start..i]);
            prev = Some(b'a');
            spaced = false;
        }
    }
    State::Code
}

fn number_end(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    if bytes[i] == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X' | b'b' | b'B')) {
        i += 2;
        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1) != Some(&b'.') {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// Emits a backtick string starting at `start`, its `{...}` parts lexed as
/// code, and returns where it ends.
fn lex_interpolated(line: &str, start: usize, emit: &mut dyn FnMut(Class, &str)) -> usize {
    let bytes = line.as_bytes();
    let mut i = start + 1;
    let mut piece = start;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => {
                i += 1;
                emit(Class::String, &line[piece..i]);
                return i;
            }
            b'{' => {
                emit(Class::String, &line[piece..i]);
                emit(Class::Operator, "{");
                let inner = i + 1;
                let mut depth = 1;
                let mut j = inner;
                while j < bytes.len() {
                    match bytes[j] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        b'`' => break,
                        _ => {}
                    }
                    j += 1;
                }
                if inner < j {
                    lex_line(&line[inner..j], State::Code, emit);
                }
                if bytes.get(j) == Some(&b'}') {
                    emit(Class::Operator, "}");
                    j += 1;
                }
                i = j;
                piece = j;
            }
            _ => i += 1,
        }
    }
    let end = i.min(bytes.len());
    if piece < end {
        emit(Class::String, &line[piece..end]);
    }
    end
}

/// The state every line of `text` starts in (one entry per `\n`-separated
/// line, so an empty text has one line).
pub fn line_states(text: &str) -> Vec<State> {
    let mut states = Vec::with_capacity(text.len() / 32 + 1);
    let mut state = State::Code;
    let mut sink = |_: Class, _: &str| {};
    for line in text.split('\n') {
        states.push(state);
        state = lex_line(line, state, &mut sink);
    }
    states
}

/// Appends `text` to `out` with the three characters HTML cares about in
/// element content escaped (and `"` for attributes).
pub fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// Appends `line`'s highlighted HTML to `out`; returns the next line's state.
pub fn render_line(line: &str, state: State, out: &mut String) -> State {
    lex_line(line, state, &mut |class, piece| match class.css() {
        Some(css) => {
            out.push_str("<span class=\"");
            out.push_str(css);
            out.push_str("\">");
            push_escaped(out, piece);
            out.push_str("</span>");
        }
        None => push_escaped(out, piece),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(line: &str, state: State) -> (Vec<(Class, String)>, State) {
        let mut out = Vec::new();
        let next = lex_line(line, state, &mut |class, piece| {
            out.push((class, piece.to_string()))
        });
        (out, next)
    }

    fn classes(line: &str) -> Vec<(Class, String)> {
        pieces(line, State::Code)
            .0
            .into_iter()
            .filter(|(class, text)| *class != Class::Text || !text.trim().is_empty())
            .collect()
    }

    fn class_of(line: &str, word: &str) -> Class {
        classes(line)
            .into_iter()
            .find(|(_, text)| text == word)
            .map(|(class, _)| class)
            .unwrap_or_else(|| panic!("{word} not in {line}"))
    }

    #[test]
    fn pieces_concatenate_back_to_the_line() {
        for line in [
            "local x = game:GetService(\"Players\") -- hi",
            "print(`a {b + 1} c`) ; ü ünïcode 日本",
            "x = [[ long ]] .. 'q\\'s' .. 0x1F .. 1e-3",
            "",
            "\t\tend",
        ] {
            let (parts, _) = pieces(line, State::Code);
            let joined: String = parts.iter().map(|(_, text)| text.as_str()).collect();
            assert_eq!(joined, line);
        }
    }

    #[test]
    fn classifies_the_usual_suspects() {
        let line = "local function spawnPart(size: Vector3): Part";
        assert_eq!(class_of(line, "local"), Class::Keyword);
        assert_eq!(class_of(line, "spawnPart"), Class::Function);
        assert_eq!(class_of(line, "Vector3"), Class::Type);
        let line = "workspace.Part:Destroy() print(self.x, nil, 42)";
        assert_eq!(class_of(line, "workspace"), Class::Builtin);
        assert_eq!(class_of(line, "Destroy"), Class::Function);
        assert_eq!(class_of(line, "Part"), Class::Text);
        assert_eq!(class_of(line, "print"), Class::Builtin);
        assert_eq!(class_of(line, "self"), Class::SelfRef);
        assert_eq!(class_of(line, "nil"), Class::Constant);
        assert_eq!(class_of(line, "42"), Class::Number);
        // A field that happens to be called like a keyword is just a field.
        assert_eq!(class_of("t.end = 1", "end"), Class::Text);
    }

    #[test]
    fn long_comments_and_strings_cross_lines() {
        let text = "a = 1 --[==[ start\nstill ]] comment\n]==] b = [[x\ny]] c";
        let states = line_states(text);
        assert_eq!(
            states,
            vec![
                State::Code,
                State::LongComment(2),
                State::LongComment(2),
                State::LongString(0)
            ]
        );
        let (parts, next) = pieces("]==] b = 2", State::LongComment(2));
        assert_eq!(parts[0], (Class::Comment, "]==]".to_string()));
        assert_eq!(next, State::Code);
    }

    #[test]
    fn interpolation_is_code_inside_a_string() {
        let parts = classes("x = `hp {math.floor(hp)}!`");
        assert!(parts.contains(&(Class::String, "`hp ".to_string())));
        assert!(parts.contains(&(Class::Builtin, "math".to_string())));
        assert!(parts.contains(&(Class::String, "!`".to_string())));
    }

    #[test]
    fn renders_escaped_spans() {
        let mut out = String::new();
        render_line("if a < b then", State::Code, &mut out);
        assert_eq!(
            out,
            "<span class=\"s-kw\">if</span> a <span class=\"s-op\">&lt;</span> b <span class=\"s-kw\">then</span>"
        );
    }

    #[test]
    fn numbers() {
        for number in [
            "0", "3.14", "1e10", "2.5E-3", "0xFF_FF", "0b1010", "1_000", ".5",
        ] {
            let parts = classes(number);
            assert_eq!(parts, vec![(Class::Number, number.to_string())], "{number}");
        }
        // `1..2` is a number, a concatenation, a number.
        assert_eq!(classes("1..2").len(), 4);
    }
}
