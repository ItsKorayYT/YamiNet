//! Small Rust guest SDK for Yami Games ABI 1 and 2. Target wasm32-unknown-unknown.
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

/// Opt-in ABI 2 extension. ABI 1 guests can continue using the root drawing API.
pub mod v2 {
    pub const ABI_VERSION: i32 = 2;
    pub const UP: u32 = 16;
    pub const DOWN: u32 = 32;
    pub const ACTION: u32 = 64;
    pub const SECONDARY: u32 = 128;
    pub const MOUSE_LEFT: u32 = 256;
    pub const MOUSE_RIGHT: u32 = 512;
    #[link(wasm_import_module = "yami_game_v2")]
    extern "C" {
        #[link_name = "sprite"]
        fn host_sprite(
            asset: u32,
            sx: i32,
            sy: i32,
            sw: i32,
            sh: i32,
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            layer: i32,
        );
        #[link_name = "input"]
        fn host_input(field: i32) -> i32;
        #[link_name = "audio"]
        fn host_audio(op: i32, channel: i32, asset: u32, value: i32, looping: i32);
    }
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Input {
        pub held: u32,
        pub pressed: u32,
        pub released: u32,
        pub x: i32,
        pub y: i32,
    }
    impl Input {
        pub fn read() -> Self {
            unsafe {
                Self {
                    held: host_input(0) as u32,
                    pressed: host_input(1) as u32,
                    released: host_input(2) as u32,
                    x: host_input(3),
                    y: host_input(4),
                }
            }
        }
        pub fn down(self, key: u32) -> bool {
            self.held & key != 0
        }
        pub fn pressed(self, key: u32) -> bool {
            self.pressed & key != 0
        }
        pub fn released(self, key: u32) -> bool {
            self.released & key != 0
        }
        pub fn pointer(self) -> Option<(i32, i32)> {
            (self.x >= 0 && self.y >= 0).then_some((self.x, self.y))
        }
    }
    /// Source rectangle in texture pixels; destination in the 640×480 game space.
    #[derive(Clone, Copy, Debug)]
    pub struct Sprite {
        pub asset: u32,
        pub source: [i32; 4],
        pub size: [i32; 2],
        pub layer: i32,
    }
    impl Sprite {
        pub fn draw(self, x: i32, y: i32) {
            let [sx, sy, sw, sh] = self.source;
            unsafe {
                host_sprite(
                    self.asset,
                    sx,
                    sy,
                    sw,
                    sh,
                    x,
                    y,
                    self.size[0],
                    self.size[1],
                    self.layer,
                )
            }
        }
        /// Advance a horizontal strip by elapsed simulation time, without allocating.
        pub fn animated(mut self, elapsed_us: u64, frame_us: u32, frames: u32) -> Self {
            if frame_us != 0 && frames != 0 {
                self.source[0] +=
                    ((elapsed_us / frame_us as u64) % frames as u64) as i32 * self.source[2];
            }
            self
        }
    }
    /// Eight replaceable channels. Volume is 0..=1000; assets are manifest IDs.
    #[derive(Clone, Copy)]
    pub struct Channel(pub u8);
    impl Channel {
        pub fn play(self, asset: u32, volume: u16, looping: bool) {
            unsafe {
                host_audio(
                    0,
                    self.0 as i32,
                    asset,
                    volume.min(1000) as i32,
                    looping as i32,
                )
            }
        }
        pub fn stop(self) {
            unsafe { host_audio(1, self.0 as i32, 0, 0, 0) }
        }
        pub fn volume(self, volume: u16) {
            unsafe { host_audio(2, self.0 as i32, 0, volume.min(1000) as i32, 0) }
        }
    }
    pub fn mute(muted: bool) {
        unsafe { host_audio(3, 0, 0, muted as i32, 0) }
    }
}
/// Export a complete ABI 2 game using a thread-local RefCell (requires guest std).
/// The state implements Default and `fn frame(&mut self, input: v2::Input, dt_us: i32)`.
#[macro_export]
macro_rules! export_game {
    ($state:ty) => {
        std::thread_local! { static YAMI_GAME: std::cell::RefCell<$state> = std::cell::RefCell::new(<$state>::default()); }
        #[no_mangle] pub extern "C" fn yami_game_abi() -> i32 { $crate::v2::ABI_VERSION }
        #[no_mangle] pub extern "C" fn yami_game_init() { yami_game_reset(); }
        #[no_mangle] pub extern "C" fn yami_game_reset() { YAMI_GAME.with(|g| *g.borrow_mut() = <$state>::default()); }
        #[no_mangle] pub extern "C" fn yami_game_input(_: u32, _: u32) {}
        #[no_mangle] pub extern "C" fn yami_game_update(dt_us: i32) { YAMI_GAME.with(|g| g.borrow_mut().frame($crate::v2::Input::read(), dt_us)); }
    };
}
