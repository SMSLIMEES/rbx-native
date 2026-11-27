//! Undo and redo for the script editor, kept as edits (where, what was
//! removed, what was inserted) rather than whole copies of the text, so a
//! long session on a big script costs little. Typing in one place within a
//! short pause is merged into a single step, as every editor does.

/// One change to the text: at byte `at`, `removed` became `inserted`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
    /// The selection (byte offsets) before and after the edit.
    pub before: (usize, usize),
    pub after: (usize, usize),
}

impl Edit {
    /// The smallest edit turning `old` into `new`: their common prefix and
    /// suffix are left out. `None` when they are equal.
    pub fn between(
        old: &str,
        new: &str,
        before: (usize, usize),
        after: (usize, usize),
    ) -> Option<Edit> {
        if old == new {
            return None;
        }
        let mut prefix = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
            prefix -= 1;
        }
        let max_suffix = old.len().min(new.len()) - prefix;
        let mut suffix = old
            .bytes()
            .rev()
            .zip(new.bytes().rev())
            .take(max_suffix)
            .take_while(|(a, b)| a == b)
            .count();
        while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix)
        {
            suffix -= 1;
        }
        Some(Edit {
            at: prefix,
            removed: old[prefix..old.len() - suffix].to_string(),
            inserted: new[prefix..new.len() - suffix].to_string(),
            before,
            after,
        })
    }

    fn apply(&self, text: &mut String) {
        text.replace_range(self.at..self.at + self.removed.len(), &self.inserted);
    }

    fn revert(&self, text: &mut String) {
        text.replace_range(self.at..self.at + self.inserted.len(), &self.removed);
    }

    /// Whether `next` continues this edit as one typing run: an insertion
    /// right where this one ended, or a backspace right where it ended.
    fn continues_with(&self, next: &Edit) -> bool {
        let end = self.at + self.inserted.len();
        let typing = self.removed.is_empty()
            && next.removed.is_empty()
            && next.at == end
            && !next.inserted.contains('\n');
        let deleting = self.inserted.is_empty()
            && next.inserted.is_empty()
            && next.at + next.removed.len() == self.at;
        typing || deleting
    }
}

/// The undo and redo stacks of one script.
#[derive(Default)]
pub struct History {
    undo: Vec<(Edit, f64)>,
    redo: Vec<Edit>,
}

/// Typing pauses longer than this start a new undo step.
const MERGE_WINDOW_MS: f64 = 1200.0;
/// Steps kept per script.
const LIMIT: usize = 500;

impl History {
    /// Records `edit`, made at `now_ms`, merging it into the last step when
    /// it continues it.
    pub fn record(&mut self, edit: Edit, now_ms: f64) {
        self.redo.clear();
        if let Some((last, at)) = self.undo.last_mut() {
            if now_ms - *at < MERGE_WINDOW_MS && last.continues_with(&edit) {
                if edit.removed.is_empty() {
                    last.inserted.push_str(&edit.inserted);
                } else {
                    last.removed.insert_str(0, &edit.removed);
                    last.at = edit.at;
                }
                last.after = edit.after;
                *at = now_ms;
                return;
            }
        }
        self.undo.push((edit, now_ms));
        if self.undo.len() > LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last step on `text`; returns the selection to restore.
    pub fn undo(&mut self, text: &mut String) -> Option<(usize, usize)> {
        let (edit, _) = self.undo.pop()?;
        edit.revert(text);
        let selection = edit.before;
        self.redo.push(edit);
        Some(selection)
    }

    /// Redoes the last undone step on `text`; returns the selection.
    pub fn redo(&mut self, text: &mut String) -> Option<(usize, usize)> {
        let edit = self.redo.pop()?;
        edit.apply(text);
        let selection = edit.after;
        // Never merged into: a redo is its own step.
        self.undo.push((edit, f64::NEG_INFINITY));
        Some(selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_into(history: &mut History, text: &mut String, at: usize, typed: &str, now: f64) {
        let old = text.clone();
        text.insert_str(at, typed);
        let edit = Edit::between(&old, text, (at, at), (at + typed.len(), at + typed.len()));
        history.record(edit.unwrap(), now);
    }

    #[test]
    fn minimal_edits() {
        let edit = Edit::between("local x = 1", "local xy = 1", (7, 7), (8, 8)).unwrap();
        assert_eq!(
            (edit.at, edit.removed.as_str(), edit.inserted.as_str()),
            (7, "", "y")
        );
        let edit = Edit::between("aaa", "aa", (3, 3), (2, 2)).unwrap();
        assert_eq!(edit.removed, "a");
        // Multi-byte characters are never split.
        let edit = Edit::between("é", "è", (0, 0), (2, 2)).unwrap();
        assert_eq!((edit.removed.as_str(), edit.inserted.as_str()), ("é", "è"));
        assert!(Edit::between("x", "x", (0, 0), (0, 0)).is_none());
    }

    #[test]
    fn typing_merges_and_undoes_in_steps() {
        let mut history = History::default();
        let mut text = String::from("print()");
        type_into(&mut history, &mut text, 6, "h", 0.0);
        type_into(&mut history, &mut text, 7, "i", 100.0);
        type_into(&mut history, &mut text, 8, "!", 5000.0);
        assert_eq!(text, "print(hi!)");
        assert_eq!(history.undo(&mut text), Some((8, 8)));
        assert_eq!(text, "print(hi)");
        assert_eq!(history.undo(&mut text), Some((6, 6)));
        assert_eq!(text, "print()");
        assert!(!history.can_undo());
        assert_eq!(history.redo(&mut text), Some((8, 8)));
        assert_eq!(text, "print(hi)");
        // A new edit drops what could have been redone.
        type_into(&mut history, &mut text, 0, "x", 9000.0);
        assert!(!history.can_redo());
    }

    #[test]
    fn backspacing_merges_too() {
        let mut history = History::default();
        let mut text = String::from("abcd");
        for (now, end) in [(0.0, 4), (50.0, 3)] {
            let old = text.clone();
            text.remove(end - 1);
            let edit = Edit::between(&old, &text, (end, end), (end - 1, end - 1)).unwrap();
            history.record(edit, now);
        }
        assert_eq!(text, "ab");
        history.undo(&mut text);
        assert_eq!(text, "abcd");
    }
}
