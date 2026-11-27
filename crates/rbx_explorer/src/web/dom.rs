//! Small conveniences over `web-sys`.

use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, HtmlTextAreaElement, Window};

pub fn window() -> Window {
    web_sys::window().expect("a window")
}

pub fn document() -> Document {
    window().document().expect("a document")
}

pub fn by_id(id: &str) -> Option<Element> {
    document().get_element_by_id(id)
}

pub fn html_by_id(id: &str) -> Option<HtmlElement> {
    by_id(id).and_then(|element| element.dyn_into().ok())
}

pub fn input(id: &str) -> Option<HtmlInputElement> {
    by_id(id).and_then(|element| element.dyn_into().ok())
}

pub fn textarea(id: &str) -> Option<HtmlTextAreaElement> {
    by_id(id).and_then(|element| element.dyn_into().ok())
}

pub fn set_html(id: &str, html: &str) {
    if let Some(element) = by_id(id) {
        element.set_inner_html(html);
    }
}

pub fn set_style(element: &Element, property: &str, value: &str) {
    if let Some(element) = element.dyn_ref::<HtmlElement>() {
        let _ = element.style().set_property(property, value);
    }
}

pub fn set_style_id(id: &str, property: &str, value: &str) {
    if let Some(element) = by_id(id) {
        set_style(&element, property, value);
    }
}

pub fn set_hidden(id: &str, hidden: bool) {
    if let Some(element) = html_by_id(id) {
        element.set_hidden(hidden);
    }
}

pub fn set_disabled(id: &str, disabled: bool) {
    if let Some(element) = by_id(id) {
        if disabled {
            let _ = element.set_attribute("disabled", "");
        } else {
            let _ = element.remove_attribute("disabled");
        }
    }
}

pub fn toggle_class(id: &str, class: &str, on: bool) {
    if let Some(element) = by_id(id) {
        let _ = element.class_list().toggle_with_force(class, on);
    }
}

pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

pub fn storage_get(key: &str) -> Option<String> {
    window()
        .local_storage()
        .ok()
        .flatten()
        .and_then(|storage| storage.get_item(key).ok().flatten())
}

pub fn storage_set(key: &str, value: &str) {
    if let Some(storage) = window().local_storage().ok().flatten() {
        let _ = storage.set_item(key, value);
    }
}

/// Runs `callback` once after `ms` milliseconds.
pub fn timeout(ms: i32, callback: impl FnOnce() + 'static) -> i32 {
    let closure = wasm_bindgen::closure::Closure::once_into_js(callback);
    window()
        .set_timeout_with_callback_and_timeout_and_arguments_0(closure.unchecked_ref(), ms)
        .unwrap_or(0)
}

pub fn clear_timeout(handle: i32) {
    window().clear_timeout_with_handle(handle);
}
