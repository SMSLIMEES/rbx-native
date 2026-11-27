//! An opened place or model: the DOM, which format it came in, and which of
//! its scripts have been edited since it was opened or last saved.

use std::collections::HashMap;

use rbx_dom::{Ref, Variant, WeakDom};
use rbx_reflection::ReflectionDatabase;

/// The two encodings Roblox saves places and models in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// `.rbxl` / `.rbxm`.
    Binary,
    /// `.rbxlx` / `.rbxmx`.
    Xml,
}

impl Format {
    /// Sniffs the bytes rather than trusting a name: a file shared from a
    /// chat app often arrives without its extension.
    pub fn detect(bytes: &[u8]) -> Format {
        if rbx_xml::is_xml(bytes) {
            Format::Xml
        } else {
            Format::Binary
        }
    }
}

/// A whole place (services at the root) or a model (anything at the root).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Place,
    Model,
}

impl Kind {
    /// The extension a file of this kind and format is saved under.
    pub fn extension(self, format: Format) -> &'static str {
        match (self, format) {
            (Kind::Place, Format::Binary) => "rbxl",
            (Kind::Place, Format::Xml) => "rbxlx",
            (Kind::Model, Format::Binary) => "rbxm",
            (Kind::Model, Format::Xml) => "rbxmx",
        }
    }
}

/// How a script runs, as the badge beside it says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptKind {
    /// A `Script` (or one whose `RunContext` is `Server`/`Legacy`).
    Server,
    /// A `LocalScript`, or a `Script` whose `RunContext` is `Client`.
    Client,
    /// A `ModuleScript`.
    Module,
}

/// Whether `class` is one of the script classes whose `Source` is Luau.
pub fn is_script_class(class: &str) -> bool {
    matches!(class, "Script" | "LocalScript" | "ModuleScript")
}

/// What kind of script `referent` is, if it is one at all.
pub fn script_kind(dom: &WeakDom, referent: Ref) -> Option<ScriptKind> {
    let instance = dom.get(referent)?;
    match instance.class() {
        "LocalScript" => Some(ScriptKind::Client),
        "ModuleScript" => Some(ScriptKind::Module),
        // `Enum.RunContext`: Legacy 0, Server 1, Client 2, Plugin 3.
        "Script" => match instance.properties().get("RunContext") {
            Some(Variant::Enum(2)) => Some(ScriptKind::Client),
            _ => Some(ScriptKind::Server),
        },
        _ => None,
    }
}

/// An opened file.
pub struct Document {
    dom: WeakDom,
    name: String,
    format: Format,
    kind: Kind,
    byte_len: usize,
    instance_count: usize,
    /// The `Source` each edited script had when it was opened or last saved,
    /// so editing a script back to what it was clears its "edited" mark.
    originals: HashMap<Ref, String>,
}

impl Document {
    /// Parses `bytes`, binary or XML, whatever `name` says.
    pub fn open(name: &str, bytes: &[u8]) -> Result<Document, String> {
        if bytes.is_empty() {
            return Err("the file is empty".to_string());
        }
        let format = Format::detect(bytes);
        let mut dom = match format {
            Format::Xml => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|err| format!("the XML is not valid UTF-8: {err}"))?;
                let text = text.strip_prefix('\u{feff}').unwrap_or(text);
                rbx_xml::deserialize(text).map_err(|err| err.to_string())?
            }
            Format::Binary => rbx_binary::deserialize(bytes).map_err(|err| err.to_string())?,
        };
        // The parser built the tree through the tracked API; that log is the
        // file's construction, not an edit.
        dom.take_changes();
        let kind = kind_of(name, &dom);
        let instance_count = count(&dom);
        Ok(Document {
            dom,
            name: name.to_string(),
            format,
            kind,
            byte_len: bytes.len(),
            instance_count,
            originals: HashMap::new(),
        })
    }

    pub fn dom(&self) -> &WeakDom {
        &self.dom
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn byte_len(&self) -> usize {
        self.byte_len
    }

    pub fn instance_count(&self) -> usize {
        self.instance_count
    }

    /// Whether anything differs from what was opened or last saved.
    pub fn is_dirty(&self) -> bool {
        !self.originals.is_empty()
    }

    /// Whether this script's `Source` differs from the saved one.
    pub fn is_edited(&self, referent: Ref) -> bool {
        self.originals.contains_key(&referent)
    }

    /// How many scripts differ from the saved file.
    pub fn edited_count(&self) -> usize {
        self.originals.len()
    }

    /// A script's `Source`. A binary file stores it as a string column, which
    /// is normally UTF-8; anything else is shown with the bad bytes replaced
    /// rather than refused.
    pub fn source(&self, referent: Ref) -> Option<String> {
        let instance = self.dom.get(referent)?;
        match instance.properties().get("Source") {
            Some(Variant::String(text)) => Some(text.clone()),
            Some(Variant::Unknown { raw, .. }) => Some(String::from_utf8_lossy(raw).into_owned()),
            Some(_) => None,
            None if is_script_class(instance.class()) => Some(String::new()),
            None => None,
        }
    }

    /// Writes a script's `Source`. Returns whether the stored text changed.
    pub fn set_source(&mut self, referent: Ref, text: &str) -> bool {
        let Some(current) = self.source(referent) else {
            return false;
        };
        if current == text {
            return false;
        }
        if self
            .dom
            .set_property(referent, "Source", Variant::String(text.to_string()))
            .is_err()
        {
            return false;
        }
        self.dom.take_changes();
        match self.originals.get(&referent) {
            Some(original) if original == text => {
                self.originals.remove(&referent);
            }
            Some(_) => {}
            None => {
                self.originals.insert(referent, current);
            }
        }
        true
    }

    /// Puts an edited script back the way the file has it.
    pub fn revert(&mut self, referent: Ref) -> Option<String> {
        let original = self.originals.remove(&referent)?;
        let _ = self
            .dom
            .set_property(referent, "Source", Variant::String(original.clone()));
        self.dom.take_changes();
        Some(original)
    }

    /// The file's bytes in `format`. Binary saves fill a property some
    /// instances of a class hold and others don't with the class default,
    /// the way `rbxstudio` saves (see `rbx_binary::serialize_with_defaults`).
    pub fn encode(&self, format: Format) -> Result<Vec<u8>, String> {
        match format {
            Format::Binary => {
                let database = ReflectionDatabase::shared();
                rbx_binary::serialize_with_defaults(&self.dom, |class, key| {
                    database.stored_default(class, key).cloned()
                })
                .map_err(|err| err.to_string())
            }
            Format::Xml => rbx_xml::serialize(&self.dom)
                .map(String::into_bytes)
                .map_err(|err| err.to_string()),
        }
    }

    /// Records a successful save: what is in memory is now what is on disk,
    /// under `name` and in `format`.
    pub fn mark_saved(&mut self, name: &str, format: Format, byte_len: usize) {
        self.originals.clear();
        self.name = name.to_string();
        self.format = format;
        self.byte_len = byte_len;
    }

    /// `name` with the extension `format` wants, for a "save a copy" picker.
    pub fn suggested_name(&self, format: Format) -> String {
        let stem = match self.name.rsplit_once('.') {
            Some((stem, ext)) if is_roblox_extension(ext) => stem,
            _ => self.name.as_str(),
        };
        let stem = if stem.is_empty() { "untitled" } else { stem };
        format!("{stem}.{}", self.kind.extension(format))
    }
}

fn is_roblox_extension(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "rbxl" | "rbxm" | "rbxlx" | "rbxmx"
    )
}

/// A place by its extension, or, for a file without one, by whether a
/// `Workspace` sits at the root.
fn kind_of(name: &str, dom: &WeakDom) -> Kind {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".rbxl") || lower.ends_with(".rbxlx") {
        return Kind::Place;
    }
    if lower.ends_with(".rbxm") || lower.ends_with(".rbxmx") {
        return Kind::Model;
    }
    let has_workspace = dom
        .root_refs()
        .iter()
        .any(|root| dom.get(*root).is_some_and(|i| i.class() == "Workspace"));
    if has_workspace {
        Kind::Place
    } else {
        Kind::Model
    }
}

fn count(dom: &WeakDom) -> usize {
    let mut stack: Vec<Ref> = dom.root_refs().to_vec();
    let mut total = 0;
    while let Some(referent) = stack.pop() {
        if let Some(instance) = dom.get(referent) {
            total += 1;
            stack.extend_from_slice(instance.children());
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Vec<u8> {
        let mut dom = WeakDom::new();
        let folder = dom.new_instance("Folder", "Stuff", None);
        let script = dom.new_instance("Script", "Main", Some(folder));
        dom.set_property(script, "Source", Variant::String("print(1)".into()))
            .unwrap();
        dom.new_instance("ModuleScript", "Lib", Some(folder));
        rbx_binary::serialize(&dom).unwrap()
    }

    fn find(doc: &Document, name: &str) -> Ref {
        let dom = doc.dom();
        let mut stack = dom.root_refs().to_vec();
        while let Some(r) = stack.pop() {
            let instance = dom.get(r).unwrap();
            if instance.name() == name {
                return r;
            }
            stack.extend_from_slice(instance.children());
        }
        panic!("no {name}");
    }

    #[test]
    fn opens_a_binary_model_and_counts_it() {
        let doc = Document::open("stuff.rbxm", &model()).unwrap();
        assert_eq!(doc.format(), Format::Binary);
        assert_eq!(doc.kind(), Kind::Model);
        assert_eq!(doc.instance_count(), 3);
        assert!(!doc.is_dirty());
    }

    #[test]
    fn editing_and_reverting_a_script_tracks_dirtiness() {
        let mut doc = Document::open("stuff.rbxm", &model()).unwrap();
        let main = find(&doc, "Main");
        assert_eq!(doc.source(main).as_deref(), Some("print(1)"));
        assert!(doc.set_source(main, "print(2)"));
        assert!(doc.is_dirty() && doc.is_edited(main));
        assert!(doc.set_source(main, "print(1)"));
        assert!(!doc.is_dirty());
        // A script without a stored Source reads as empty and can be written.
        let lib = find(&doc, "Lib");
        assert_eq!(doc.source(lib).as_deref(), Some(""));
        assert!(doc.set_source(lib, "return {}"));
        assert_eq!(doc.edited_count(), 1);
    }

    #[test]
    fn saved_bytes_reopen_with_the_edit_in_both_formats() {
        let mut doc = Document::open("stuff.rbxm", &model()).unwrap();
        let main = find(&doc, "Main");
        doc.set_source(main, "print(\"edited\")");
        for format in [Format::Binary, Format::Xml] {
            let bytes = doc.encode(format).unwrap();
            assert_eq!(Format::detect(&bytes), format);
            let reopened = Document::open(&doc.suggested_name(format), &bytes).unwrap();
            let main = find(&reopened, "Main");
            assert_eq!(reopened.source(main).as_deref(), Some("print(\"edited\")"));
            assert_eq!(reopened.instance_count(), 3);
        }
    }

    #[test]
    fn suggested_names_swap_the_extension() {
        let doc = Document::open("My Place.rbxl", &model()).unwrap();
        assert_eq!(doc.suggested_name(Format::Xml), "My Place.rbxlx");
        assert_eq!(doc.suggested_name(Format::Binary), "My Place.rbxl");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(Document::open("x.rbxl", b"definitely not a place").is_err());
        assert!(Document::open("x.rbxl", b"").is_err());
    }
}
