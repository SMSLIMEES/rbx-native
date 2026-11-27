//! The Explorer tree as a flat list of rows: what is expanded, how siblings
//! are ordered (exactly as Studio orders them), and the filtered view a
//! search or the "scripts only" chip shows. Pure data over a [`WeakDom`];
//! the screen draws only the rows in view (see `ui::tree_rows`).

mod order;
pub mod services;

use std::cmp::Ordering;
use std::collections::HashSet;

use rbx_dom::{Ref, WeakDom};

use crate::document::is_script_class;
use services::{explorer_rank, is_default_visible};

/// Where a class sits among its siblings when Studio's metadata names no
/// `ExplorerOrder` for it: after every class that does.
const UNLISTED_ORDER: i32 = 1000;

fn explorer_order(class: &str) -> i32 {
    order::EXPLORER_ORDER
        .binary_search_by(|(name, _)| name.cmp(&class))
        .map_or(UNLISTED_ORDER, |index| order::EXPLORER_ORDER[index].1)
}

/// `parent`'s children in Studio's order: by the class's `ExplorerOrder`,
/// then by name byte by byte; stable, so same-named siblings keep the file's
/// order.
pub fn sorted_children(dom: &WeakDom, parent: Ref) -> Vec<Ref> {
    let Some(instance) = dom.get(parent) else {
        return Vec::new();
    };
    let mut children: Vec<(i32, &str, Ref)> = instance
        .children()
        .iter()
        .filter_map(|child| {
            let child_instance = dom.get(*child)?;
            Some((
                explorer_order(child_instance.class()),
                child_instance.name(),
                *child,
            ))
        })
        .collect();
    children.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.as_bytes().cmp(right.1.as_bytes()))
    });
    children.into_iter().map(|(_, _, child)| child).collect()
}

/// The roots in Studio's order: services by their fixed rank first, then
/// everything else by name, case-insensitively.
pub fn sorted_roots(dom: &WeakDom, all_services: bool) -> Vec<Ref> {
    let mut roots: Vec<(Option<usize>, &str, Ref)> = dom
        .root_refs()
        .iter()
        .filter_map(|root| {
            let instance = dom.get(*root)?;
            (all_services || is_default_visible(instance.class())).then_some((
                explorer_rank(instance.class()),
                instance.name(),
                *root,
            ))
        })
        .collect();
    roots.sort_by(|left, right| match (left.0, right.0) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => left
            .1
            .to_lowercase()
            .cmp(&right.1.to_lowercase())
            .then_with(|| left.1.cmp(right.1)),
    });
    roots.into_iter().map(|(_, _, root)| root).collect()
}

/// The byte range of the first case-insensitive occurrence of `needle` in
/// `haystack` (Unicode simple lowercasing, char by char, so the range is a
/// range of `haystack` itself). `needle` must already be lowercase.
pub fn find_folded(haystack: &str, needle: &str) -> Option<(usize, usize)> {
    if needle.is_empty() {
        return None;
    }
    let folded: Vec<char> = needle.chars().collect();
    let chars: Vec<(usize, char)> = haystack.char_indices().collect();
    'start: for start in 0..chars.len() {
        if chars.len() - start < folded.len() {
            break;
        }
        for (offset, want) in folded.iter().enumerate() {
            let have = chars[start + offset].1;
            if have != *want && have.to_lowercase().next() != Some(*want) {
                continue 'start;
            }
        }
        let begin = chars[start].0;
        let end = chars
            .get(start + folded.len())
            .map_or(haystack.len(), |(index, _)| *index);
        return Some((begin, end));
    }
    None
}

/// One line of the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub referent: Ref,
    pub depth: u16,
    /// Whether it has children at all (and so draws a chevron).
    pub has_children: bool,
    pub expanded: bool,
    /// In a filtered view: whether this row matched (the others are the
    /// ancestors that lead to a match).
    pub matched: bool,
}

/// The tree's view state.
#[derive(Default)]
pub struct Explorer {
    expanded: HashSet<Ref>,
    all_services: bool,
    scripts_only: bool,
    /// Lowercased.
    query: String,
    rows: Vec<Row>,
    matches: usize,
}

impl Explorer {
    /// A fresh view of `dom`: a model's lone root starts expanded, a
    /// place starts with everything collapsed, as Studio opens one.
    pub fn new(dom: &WeakDom) -> Explorer {
        let mut explorer = Explorer::default();
        let roots = sorted_roots(dom, false);
        let is_place = roots.iter().any(|root| {
            dom.get(*root)
                .is_some_and(|instance| services::is_known_service(instance.class()))
        });
        if !is_place && roots.len() <= 3 {
            explorer.expanded.extend(roots);
        }
        explorer.rebuild(dom);
        explorer
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Whether a search or the scripts chip is narrowing the tree.
    pub fn is_filtered(&self) -> bool {
        !self.query.is_empty() || self.scripts_only
    }

    /// Matches in the filtered view.
    pub fn match_count(&self) -> usize {
        self.matches
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn all_services(&self) -> bool {
        self.all_services
    }

    pub fn scripts_only(&self) -> bool {
        self.scripts_only
    }

    pub fn set_query(&mut self, dom: &WeakDom, query: &str) {
        let query = query.trim().to_lowercase();
        if query != self.query {
            self.query = query;
            self.rebuild(dom);
        }
    }

    pub fn set_scripts_only(&mut self, dom: &WeakDom, on: bool) {
        self.scripts_only = on;
        self.rebuild(dom);
    }

    pub fn set_all_services(&mut self, dom: &WeakDom, on: bool) {
        self.all_services = on;
        self.rebuild(dom);
    }

    pub fn is_expanded(&self, referent: Ref) -> bool {
        self.expanded.contains(&referent)
    }

    pub fn toggle(&mut self, dom: &WeakDom, referent: Ref) {
        if !self.expanded.remove(&referent) {
            self.expanded.insert(referent);
        }
        self.rebuild(dom);
    }

    pub fn collapse_all(&mut self, dom: &WeakDom) {
        self.expanded.clear();
        self.rebuild(dom);
    }

    /// Expands every ancestor of `referent` so it has a row, clearing a
    /// filter that would hide it; returns its row index.
    pub fn reveal(&mut self, dom: &WeakDom, referent: Ref) -> Option<usize> {
        let mut parent = dom.parent(referent);
        while let Some(ancestor) = parent {
            self.expanded.insert(ancestor);
            parent = dom.parent(ancestor);
        }
        if let Some(root) = ancestors_and_self(dom, referent).last() {
            let hidden_root = dom
                .get(*root)
                .is_some_and(|instance| !is_default_visible(instance.class()));
            if hidden_root {
                self.all_services = true;
            }
        }
        self.query.clear();
        self.scripts_only = false;
        self.rebuild(dom);
        self.index_of(referent)
    }

    pub fn index_of(&self, referent: Ref) -> Option<usize> {
        self.rows.iter().position(|row| row.referent == referent)
    }

    /// Recomputes the rows from the DOM and the view state.
    pub fn rebuild(&mut self, dom: &WeakDom) {
        self.rows.clear();
        self.matches = 0;
        if self.is_filtered() {
            // A search looks everywhere, hidden services included: a script
            // the user is hunting for must be found wherever it lives.
            let roots = sorted_roots(dom, true);
            let mut rows = Vec::new();
            let mut matches = 0;
            for root in roots {
                self.filtered(dom, root, 0, &mut rows, &mut matches);
            }
            self.rows = rows;
            self.matches = matches;
        } else {
            let roots = sorted_roots(dom, self.all_services);
            let mut rows = Vec::new();
            for root in roots {
                self.expanded_rows(dom, root, 0, &mut rows);
            }
            self.rows = rows;
        }
    }

    fn expanded_rows(&self, dom: &WeakDom, referent: Ref, depth: u16, rows: &mut Vec<Row>) {
        let Some(instance) = dom.get(referent) else {
            return;
        };
        let has_children = !instance.children().is_empty();
        let expanded = has_children && self.expanded.contains(&referent);
        rows.push(Row {
            referent,
            depth,
            has_children,
            expanded,
            matched: false,
        });
        if expanded {
            for child in sorted_children(dom, referent) {
                self.expanded_rows(dom, child, depth.saturating_add(1), rows);
            }
        }
    }

    fn matches(&self, dom: &WeakDom, referent: Ref) -> bool {
        let Some(instance) = dom.get(referent) else {
            return false;
        };
        if self.scripts_only && !is_script_class(instance.class()) {
            return false;
        }
        self.query.is_empty()
            || find_folded(instance.name(), &self.query).is_some()
            || instance.class().to_lowercase() == self.query
    }

    /// Emits `referent`'s row and its matching descendants' if anything in
    /// its subtree matches; returns whether it did.
    fn filtered(
        &self,
        dom: &WeakDom,
        referent: Ref,
        depth: u16,
        rows: &mut Vec<Row>,
        matches: &mut usize,
    ) -> bool {
        let Some(instance) = dom.get(referent) else {
            return false;
        };
        let at = rows.len();
        let matched = self.matches(dom, referent);
        rows.push(Row {
            referent,
            depth,
            has_children: !instance.children().is_empty(),
            expanded: false,
            matched,
        });
        let mut any_child = false;
        for child in sorted_children(dom, referent) {
            any_child |= self.filtered(dom, child, depth.saturating_add(1), rows, matches);
        }
        if matched {
            *matches += 1;
        }
        if any_child {
            rows[at].expanded = true;
        }
        if !matched && !any_child {
            rows.truncate(at);
            return false;
        }
        true
    }
}

/// `referent`, its parent, ... up to its root.
pub fn ancestors_and_self(dom: &WeakDom, referent: Ref) -> Vec<Ref> {
    let mut chain = vec![referent];
    let mut parent = dom.parent(referent);
    while let Some(ancestor) = parent {
        chain.push(ancestor);
        parent = dom.parent(ancestor);
    }
    chain
}

/// The names from the root down to `referent`.
pub fn path_names(dom: &WeakDom, referent: Ref) -> Vec<String> {
    let mut names: Vec<String> = ancestors_and_self(dom, referent)
        .into_iter()
        .filter_map(|reference| dom.get(reference).map(|i| i.name().to_string()))
        .collect();
    names.reverse();
    names
}

/// Every script in the file in Explorer order, hidden services included.
pub fn all_scripts(dom: &WeakDom) -> Vec<Ref> {
    fn visit(dom: &WeakDom, referent: Ref, out: &mut Vec<Ref>) {
        if dom
            .get(referent)
            .is_some_and(|instance| is_script_class(instance.class()))
        {
            out.push(referent);
        }
        for child in sorted_children(dom, referent) {
            visit(dom, child, out);
        }
    }
    let mut out = Vec::new();
    for root in sorted_roots(dom, true) {
        visit(dom, root, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place() -> (WeakDom, Ref, Ref) {
        let mut dom = WeakDom::new();
        let lighting = dom.new_instance("Lighting", "Lighting", None);
        let workspace = dom.new_instance("Workspace", "Workspace", None);
        dom.new_instance("Chat", "Chat", None);
        let map = dom.new_instance("Folder", "Map", Some(workspace));
        dom.new_instance("Part", "zeta", Some(map));
        dom.new_instance("Part", "Alpha", Some(map));
        let script = dom.new_instance("Script", "KillBrick", Some(map));
        dom.new_instance("Camera", "Camera", Some(workspace));
        dom.new_instance("Sky", "Sky", Some(lighting));
        (dom, workspace, script)
    }

    fn names(dom: &WeakDom, explorer: &Explorer) -> Vec<String> {
        explorer
            .rows()
            .iter()
            .map(|row| {
                format!(
                    "{}{}",
                    "  ".repeat(usize::from(row.depth)),
                    dom.get(row.referent).unwrap().name()
                )
            })
            .collect()
    }

    #[test]
    fn roots_follow_studio_and_hide_services() {
        let (dom, _, _) = place();
        let explorer = Explorer::new(&dom);
        assert_eq!(names(&dom, &explorer), vec!["Workspace", "Lighting"]);
    }

    #[test]
    fn expanding_orders_children_like_studio() {
        let (dom, workspace, script) = place();
        let mut explorer = Explorer::new(&dom);
        explorer.toggle(&dom, workspace);
        let map = dom.parent(script).unwrap();
        explorer.toggle(&dom, map);
        assert_eq!(
            names(&dom, &explorer),
            vec![
                "Workspace",
                "  Camera",
                "  Map",
                "    KillBrick",
                "    Alpha",
                "    zeta",
                "Lighting"
            ]
        );
        explorer.collapse_all(&dom);
        assert_eq!(explorer.rows().len(), 2);
    }

    #[test]
    fn search_keeps_ancestors_and_counts_matches() {
        let (dom, _, _) = place();
        let mut explorer = Explorer::new(&dom);
        explorer.set_query(&dom, "ALP");
        assert_eq!(
            names(&dom, &explorer),
            vec!["Workspace", "  Map", "    Alpha"]
        );
        assert_eq!(explorer.match_count(), 1);
        assert!(explorer.rows()[0].expanded && !explorer.rows()[0].matched);
        explorer.set_query(&dom, "");
        explorer.set_scripts_only(&dom, true);
        assert_eq!(
            names(&dom, &explorer),
            vec!["Workspace", "  Map", "    KillBrick"]
        );
        // A search by class name.
        explorer.set_scripts_only(&dom, false);
        explorer.set_query(&dom, "part");
        assert_eq!(explorer.match_count(), 2);
    }

    #[test]
    fn reveal_expands_the_way_down() {
        let (dom, _, script) = place();
        let mut explorer = Explorer::new(&dom);
        explorer.set_query(&dom, "sky");
        let index = explorer.reveal(&dom, script).unwrap();
        assert_eq!(explorer.rows()[index].referent, script);
        assert!(!explorer.is_filtered());
        assert_eq!(
            path_names(&dom, script),
            vec!["Workspace", "Map", "KillBrick"]
        );
        assert_eq!(all_scripts(&dom), vec![script]);
    }

    #[test]
    fn folded_find_handles_cyrillic() {
        assert_eq!(find_folded("ГлавныйСкрипт", "скрипт"), Some((14, 26)));
        assert_eq!(find_folded("Part", "art"), Some((1, 4)));
        assert_eq!(find_folded("Part", "x"), None);
    }
}
