//! WobbleWorks, native and web. The native shell runs through eframe (wgpu); the web shell is the
//! same app compiled to WebAssembly (`trunk build --release` in this directory).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    native::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(test)]
mod tests {
    /// The web page has no script of its own (trunk injects the loader), names the canvas the
    /// app looks for, and offers a reload if the WebAssembly never arrives.
    #[test]
    fn web_page_is_script_free_and_recoverable() {
        let html = include_str!("../index.html");
        assert!(!html.contains("<script"));
        assert!(html.contains("id=\"wobbleworks_canvas\"") && html.contains("id=\"wobbleworks_loading\""));
        assert!(html.contains("<a href=\"\">Reload</a>"));
        assert!(html.contains("data-bin=\"wobbleworks\""));
    }
}
