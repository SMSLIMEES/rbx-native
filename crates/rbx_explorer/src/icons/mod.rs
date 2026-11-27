//! Which icon, and which tonal container (`k-*` in `web/style.css`), a class
//! is drawn with in the Explorer. Monochrome Material Symbols tinted by class
//! family, so a script, a part and a GUI object read apart at a glance
//! without leaving the theme's palette.

pub mod symbols;

use rbx_reflection::ReflectionDatabase;

/// The family a class belongs to, which picks its icon container colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Script,
    LocalScript,
    Module,
    Service,
    Container,
    Part,
    Gui,
    Value,
    Effect,
    Other,
}

impl Family {
    pub fn css(self) -> &'static str {
        match self {
            Family::Script => "k-script",
            Family::LocalScript => "k-local",
            Family::Module => "k-module",
            Family::Service => "k-service",
            Family::Container => "k-container",
            Family::Part => "k-part",
            Family::Gui => "k-gui",
            Family::Value => "k-value",
            Family::Effect => "k-fx",
            Family::Other => "k-other",
        }
    }
}

/// Exact classes first; the first matching superclass rule after.
const EXACT: &[(&str, &str, Family)] = &[
    ("Script", symbols::C_SCRIPT, Family::Script),
    ("LocalScript", symbols::C_LOCAL_SCRIPT, Family::LocalScript),
    ("ModuleScript", symbols::C_MODULE_SCRIPT, Family::Module),
    ("Workspace", symbols::C_PUBLIC, Family::Service),
    ("Players", symbols::C_GROUP, Family::Service),
    ("Lighting", symbols::C_SUN, Family::Service),
    ("MaterialService", symbols::C_TEXTURE, Family::Service),
    ("ReplicatedFirst", symbols::C_BOLT, Family::Service),
    ("ReplicatedStorage", symbols::C_INVENTORY, Family::Service),
    ("ServerScriptService", symbols::C_DNS, Family::Service),
    ("ServerStorage", symbols::C_DATABASE, Family::Service),
    ("StarterGui", symbols::C_DASHBOARD, Family::Service),
    ("StarterPack", symbols::C_BACKPACK, Family::Service),
    ("StarterPlayer", symbols::C_HUMAN, Family::Service),
    ("StarterPlayerScripts", symbols::C_FOLDER, Family::Container),
    (
        "StarterCharacterScripts",
        symbols::C_FOLDER,
        Family::Container,
    ),
    ("Teams", symbols::C_GROUPS, Family::Service),
    ("Team", symbols::C_FLAG, Family::Value),
    ("SoundService", symbols::C_VOLUME, Family::Service),
    ("SoundGroup", symbols::C_VOLUME, Family::Container),
    ("Sound", symbols::C_MUSIC, Family::Effect),
    ("TextChatService", symbols::C_CHAT, Family::Service),
    ("Chat", symbols::C_CHAT, Family::Service),
    ("Folder", symbols::C_FOLDER, Family::Container),
    ("Configuration", symbols::C_SETTINGS, Family::Container),
    ("Model", symbols::C_CUBE, Family::Container),
    ("Actor", symbols::C_CUBE, Family::Container),
    ("WorldModel", symbols::C_CUBE, Family::Container),
    ("Tool", symbols::C_TOOL, Family::Container),
    ("Accessory", symbols::C_CHECKROOM, Family::Container),
    ("Accoutrement", symbols::C_CHECKROOM, Family::Container),
    ("Hat", symbols::C_CHECKROOM, Family::Container),
    ("Shirt", symbols::C_CHECKROOM, Family::Value),
    ("Pants", symbols::C_CHECKROOM, Family::Value),
    ("ShirtGraphic", symbols::C_CHECKROOM, Family::Value),
    ("Humanoid", symbols::C_PERSON, Family::Other),
    ("HumanoidDescription", symbols::C_PERSON, Family::Value),
    ("Camera", symbols::C_VIDEOCAM, Family::Other),
    ("Terrain", symbols::C_TERRAIN, Family::Part),
    ("TerrainRegion", symbols::C_LANDSCAPE, Family::Value),
    ("SpawnLocation", symbols::C_FLAG, Family::Part),
    ("Seat", symbols::C_CHAIR, Family::Part),
    ("VehicleSeat", symbols::C_CHAIR, Family::Part),
    ("UnionOperation", symbols::C_JOIN, Family::Part),
    ("NegateOperation", symbols::C_JOIN, Family::Part),
    ("IntersectOperation", symbols::C_JOIN, Family::Part),
    ("MeshPart", symbols::C_CATEGORY, Family::Part),
    ("WedgePart", symbols::C_SHAPES, Family::Part),
    ("CornerWedgePart", symbols::C_SHAPES, Family::Part),
    ("TrussPart", symbols::C_GRID, Family::Part),
    ("SpecialMesh", symbols::C_CATEGORY, Family::Value),
    ("BlockMesh", symbols::C_CATEGORY, Family::Value),
    ("Decal", symbols::C_IMAGE, Family::Value),
    ("Texture", symbols::C_WALLPAPER, Family::Value),
    ("SurfaceAppearance", symbols::C_PALETTE, Family::Value),
    ("MaterialVariant", symbols::C_PALETTE, Family::Value),
    ("ScreenGui", symbols::C_DASHBOARD, Family::Gui),
    ("SurfaceGui", symbols::C_WEB, Family::Gui),
    ("BillboardGui", symbols::C_WEB, Family::Gui),
    ("Frame", symbols::C_FRAME, Family::Gui),
    ("ScrollingFrame", symbols::C_SWAP_VERT, Family::Gui),
    ("CanvasGroup", symbols::C_FRAME, Family::Gui),
    ("ViewportFrame", symbols::C_VIDEOCAM, Family::Gui),
    ("TextLabel", symbols::C_TITLE, Family::Gui),
    ("TextButton", symbols::C_BUTTON, Family::Gui),
    ("ImageButton", symbols::C_BUTTON, Family::Gui),
    ("ImageLabel", symbols::C_IMAGE, Family::Gui),
    ("TextBox", symbols::C_TEXT_FIELDS, Family::Gui),
    ("UICorner", symbols::C_ROUNDED_CORNER, Family::Value),
    ("RemoteEvent", symbols::C_CELL_TOWER, Family::Value),
    (
        "UnreliableRemoteEvent",
        symbols::C_CELL_TOWER,
        Family::Value,
    ),
    ("RemoteFunction", symbols::C_SYNC, Family::Value),
    ("BindableEvent", symbols::C_NOTIFICATIONS, Family::Value),
    ("BindableFunction", symbols::C_FUNCTIONS, Family::Value),
    ("Attachment", symbols::C_LOCATION, Family::Value),
    ("Bone", symbols::C_LOCATION, Family::Value),
    ("Animation", symbols::C_RUN, Family::Value),
    ("Animator", symbols::C_ANIMATION, Family::Value),
    ("AnimationController", symbols::C_ANIMATION, Family::Value),
    ("KeyframeSequence", symbols::C_ANIMATION, Family::Value),
    ("Sky", symbols::C_WALLPAPER, Family::Effect),
    ("Atmosphere", symbols::C_FOGGY, Family::Effect),
    ("Clouds", symbols::C_CLOUD, Family::Effect),
    ("Fire", symbols::C_FIRE, Family::Effect),
    ("Smoke", symbols::C_CLOUD, Family::Effect),
    ("Sparkles", symbols::C_SPARKLES, Family::Effect),
    ("ParticleEmitter", symbols::C_SPARKLES, Family::Effect),
    ("Beam", symbols::C_HIGHLIGHT, Family::Effect),
    ("Trail", symbols::C_SPEED, Family::Effect),
    ("Highlight", symbols::C_HIGHLIGHT, Family::Effect),
    ("Explosion", symbols::C_STAR, Family::Effect),
    ("ProximityPrompt", symbols::C_TOUCH, Family::Value),
    ("ClickDetector", symbols::C_TOUCH, Family::Value),
    ("DragDetector", symbols::C_TOUCH, Family::Value),
    ("PackageLink", symbols::C_LINK, Family::Value),
    ("ObjectValue", symbols::C_LINK, Family::Value),
    ("Weld", symbols::C_LINK, Family::Value),
    ("WeldConstraint", symbols::C_LINK, Family::Value),
    ("Motor6D", symbols::C_LINK, Family::Value),
    ("Pose", symbols::C_PIN, Family::Value),
];

/// Superclass rules, tried in order after [`EXACT`].
const ANCESTORS: &[(&str, &str, Family)] = &[
    ("BasePart", symbols::C_PART, Family::Part),
    ("PostEffect", symbols::C_BLUR, Family::Effect),
    ("Light", symbols::C_LIGHTBULB, Family::Effect),
    ("GuiObject", symbols::C_FRAME, Family::Gui),
    ("LayerCollector", symbols::C_DASHBOARD, Family::Gui),
    ("UIComponent", symbols::C_TUNE, Family::Value),
    ("ValueBase", symbols::C_VARIABLES, Family::Value),
    ("Constraint", symbols::C_LINK, Family::Value),
    ("JointInstance", symbols::C_LINK, Family::Value),
    ("BodyMover", symbols::C_SPEED, Family::Value),
    ("LuaSourceContainer", symbols::C_SCRIPT, Family::Script),
    ("Clothing", symbols::C_CHECKROOM, Family::Value),
    ("DataModelMesh", symbols::C_CATEGORY, Family::Value),
    ("PVInstance", symbols::C_CUBE, Family::Container),
];

/// The icon path and family of a row of `class`.
pub fn class_icon(class: &str) -> (&'static str, Family) {
    if let Some((_, icon, family)) = EXACT.iter().find(|(name, _, _)| *name == class) {
        return (icon, *family);
    }
    let database = ReflectionDatabase::shared();
    for (ancestor, icon, family) in ANCESTORS {
        if database.is_subclass_of(class, ancestor) {
            return (icon, *family);
        }
    }
    if crate::explorer::services::is_known_service(class) {
        return (symbols::C_SETTINGS, Family::Service);
    }
    (symbols::C_EXTENSION, Family::Other)
}

/// `<svg class="{class}">` holding the icon `path`.
pub fn svg(path: &str, class: &str) -> String {
    format!("<svg class=\"{class}\" viewBox=\"0 -960 960 960\"><path d=\"{path}\"/></svg>")
}

/// The usual 24px icon.
pub fn icon(path: &str) -> String {
    svg(path, "i")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        assert_eq!(class_icon("Script").1, Family::Script);
        assert_eq!(class_icon("Part").1, Family::Part);
        assert_eq!(class_icon("Part").0, symbols::C_PART);
        assert_eq!(class_icon("PointLight").1, Family::Effect);
        assert_eq!(class_icon("StringValue").1, Family::Value);
        assert_eq!(class_icon("UIListLayout").1, Family::Value);
        assert_eq!(class_icon("Workspace").1, Family::Service);
        assert_eq!(class_icon("ImageLabel").1, Family::Gui);
        assert_eq!(class_icon("NoSuchClassAnywhere").1, Family::Other);
    }
}
