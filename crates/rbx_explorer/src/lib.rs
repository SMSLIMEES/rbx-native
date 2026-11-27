//! RBX Explorer: a phone app to browse Roblox places and models (`.rbxl`,
//! `.rbxm`, `.rbxlx`, `.rbxmx`) in Studio's Explorer hierarchy, inspect
//! properties, and read and edit their scripts, in Material 3.
//!
//! Everything but the DOM glue is plain Rust tested on every target:
//! [`document`] opens and saves files, [`explorer`] orders and filters the
//! tree, [`highlight`] colours Luau, [`ui`] writes the screens' HTML. The
//! wasm side (`web`, only on wasm32) wires it to a WebView page.

pub mod document;
pub mod explorer;
pub mod highlight;
pub mod history;
pub mod icons;
pub mod props;
pub mod textops;
pub mod ui;

#[cfg(target_arch = "wasm32")]
mod web;
