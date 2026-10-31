//! Rust guest helpers for Yami Games ABI 1. Build only for wasm32-unknown-unknown.
#![no_std]
pub const LEFT: u32 = 1;
pub const RIGHT: u32 = 2;
pub const START: u32 = 4;
pub const RESET: u32 = 8;
#[link(wasm_import_module = "yami_game_v1")]
extern "C" {
    #[link_name = "clear"]
    fn host_clear(color: u32);
    #[link_name = "rect"]
    fn host_rect(x: i32, y: i32, w: i32, h: i32, color: u32);
    #[link_name = "circle"]
    fn host_circle(x: i32, y: i32, radius: i32, color: u32);
    #[link_name = "text"]
    fn host_text(ptr: *const u8, len: usize, x: i32, y: i32, size: i32, color: u32);
}
// These are calls into the sandbox host, not access to native OS functions. The
// host validates all coordinates and copies bounded UTF-8 from guest memory.
pub fn clear(color: u32) {
    unsafe { host_clear(color) }
}
pub fn rect(x: i32, y: i32, w: i32, h: i32, color: u32) {
    unsafe { host_rect(x, y, w, h, color) }
}
pub fn circle(x: i32, y: i32, radius: i32, color: u32) {
    unsafe { host_circle(x, y, radius, color) }
}
pub fn text(text: &str, x: i32, y: i32, size: i32, color: u32) {
    unsafe { host_text(text.as_ptr(), text.len(), x, y, size, color) }
}
/// Colors are 0xRRGGBBAA.
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 255
}
