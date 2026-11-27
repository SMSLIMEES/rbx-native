#!/usr/bin/env bash
# Builds RBX Explorer (the phone app for browsing places/models and editing
# their scripts) into crates/rbx_explorer/web/pkg, next to the page that
# loads it. Needs the wasm target and a wasm-bindgen CLI of the same version
# as the `wasm-bindgen` crate in Cargo.lock:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version <that version> --locked
# Then either serve the page from any static server, e.g.
#   python3 -m http.server -d crates/rbx_explorer/web 8080
# or build the Android app around it (packaging/android-explorer).
set -euo pipefail
cd "$(dirname "$0")/.."
target="${CARGO_TARGET_DIR:-target}"
pkg=crates/rbx_explorer/web/pkg
# `cargo rustc` rather than a `cdylib` in Cargo.toml: every native build of
# the crate would otherwise link a shared library nobody loads.
cargo rustc -p rbx_explorer --lib --release --target wasm32-unknown-unknown --crate-type cdylib
wasm-bindgen --target web --no-typescript \
  --out-dir "$pkg" \
  "$target/wasm32-unknown-unknown/release/rbx_explorer.wasm"
# The editor's font (OFL, see assets/fonts/OFL-JetBrainsMono.txt).
cp assets/fonts/JetBrainsMono-Regular.ttf assets/fonts/OFL-JetBrainsMono.txt "$pkg/"
echo "built crates/rbx_explorer/web — serve it with: python3 -m http.server -d crates/rbx_explorer/web 8080"
