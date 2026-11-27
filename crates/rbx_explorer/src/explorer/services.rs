//! Roblox Studio's Explorer root list, as data: the order it lists services
//! in and which of Roblox's own services its default view hides. Mirrors
//! `rbx_viewer::services` (the viewer pulls in wgpu, which a phone app that
//! renders nothing has no use for).

/// The services Studio's default Explorer view shows.
const SERVICE_ORDER: [&str; 14] = [
    "Workspace",
    "Players",
    "Lighting",
    "MaterialService",
    "ReplicatedFirst",
    "ReplicatedStorage",
    "ServerScriptService",
    "ServerStorage",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "Teams",
    "SoundService",
    "TextChatService",
];

/// The order Studio lists root services in, default-visible or not.
const EXPLORER_ORDER: [&str; 19] = [
    "Workspace",
    "Players",
    "Lighting",
    "MaterialService",
    "ReplicatedFirst",
    "ReplicatedStorage",
    "ServerScriptService",
    "ServerStorage",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "Teams",
    "SoundService",
    "Chat",
    "TextChatService",
    "VoiceChatService",
    "LocalizationService",
    "TestService",
    "VRService",
];

/// Every root service class Roblox itself creates in a place file.
const KNOWN_SERVICES: [&str; 55] = [
    "AssetService",
    "Chat",
    "CollectionService",
    "ContextActionService",
    "CookiesService",
    "CSGDictionaryService",
    "DataStoreService",
    "Debris",
    "DevPackages",
    "GamePassService",
    "GuidRegistryService",
    "HttpService",
    "InsertService",
    "Instance",
    "Lighting",
    "LocalizationService",
    "LodDataService",
    "LuaWebService",
    "MaterialService",
    "NonReplicatedCSGDictionaryService",
    "Packages",
    "PermissionsService",
    "PhysicsService",
    "PlayerEmulatorService",
    "Players",
    "ProcessInstancePhysicsService",
    "ProximityPromptService",
    "ReplicatedFirst",
    "ReplicatedStorage",
    "ScriptService",
    "Selection",
    "SerializationService",
    "ServerPackages",
    "ServerScriptService",
    "ServerStorage",
    "ServiceVisibilityService",
    "SoundService",
    "StarterGui",
    "StarterPack",
    "StarterPlayer",
    "StudioData",
    "Teams",
    "TeleportService",
    "TestService",
    "TextChatService",
    "TimerService",
    "TouchInputService",
    "TweenService",
    "UGCAvatarService",
    "VideoCaptureService",
    "VideoService",
    "VirtualInputManager",
    "VoiceChatService",
    "VRService",
    "Workspace",
];

/// Where a root of `class` sits in the Explorer's root list, if it has a
/// fixed place there.
pub fn explorer_rank(class: &str) -> Option<usize> {
    EXPLORER_ORDER.iter().position(|service| *service == class)
}

/// Whether a root of this class is one of Roblox's own services.
pub fn is_known_service(class: &str) -> bool {
    SERVICE_ORDER.contains(&class)
        || KNOWN_SERVICES.contains(&class)
        || rbx_reflection::ReflectionDatabase::shared().is_service(class)
}

/// Whether Studio's default Explorer view shows a root of this class without
/// "show all services": its 14 fixed services, or anything that is not one of
/// Roblox's own services at all. `AvatarSettings` is never listed.
pub fn is_default_visible(class: &str) -> bool {
    class != "AvatarSettings" && (SERVICE_ORDER.contains(&class) || !is_known_service(class))
}
