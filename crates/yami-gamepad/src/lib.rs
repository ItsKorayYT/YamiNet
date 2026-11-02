//! Small, bounded native gamepad adapter for Yami Browser.
//!
//! The guest never sees a device handle, axis value or native capability. This
//! crate only converts up to four physical pads into the same digital input bits
//! as the keyboard. The browser applies those bits to the active game session and
//! is responsible for suspending them when the game is paused or loses focus.
//!
//! Windows uses XInput (four slots). Linux reads legacy `/dev/input/js*`
//! joystick devices in non-blocking mode, so it needs no privileged ioctls and no
//! extra native library. Other platforms compile to a silent no-op backend.
//!
//! All mapping is deterministic and tested with an injectable [`Backend`].

pub const MAX_PADS: usize = 4;
/// Bit aligned with Yami Games ABI 2 (`yami_game_v2::input`).
pub const INPUT_MASK: u32 = 1023;

pub const LEFT: u32 = 1;
pub const RIGHT: u32 = 2;
pub const START: u32 = 4;
pub const RESET: u32 = 8;
pub const UP: u32 = 16;
pub const DOWN: u32 = 32;
pub const ACTION: u32 = 64;
pub const SECONDARY: u32 = 128;

/// Abstract buttons shared by every backend. Native backends translate their
/// button numbers into this mask; [`map_pad`] then maps them to game bits.
pub mod buttons {
    pub const A: u16 = 1 << 0;
    pub const B: u16 = 1 << 1;
    pub const X: u16 = 1 << 2;
    pub const Y: u16 = 1 << 3;
    pub const LEFT_SHOULDER: u16 = 1 << 4;
    pub const RIGHT_SHOULDER: u16 = 1 << 5;
    pub const BACK: u16 = 1 << 6;
    pub const START: u16 = 1 << 7;
    pub const LEFT_THUMB: u16 = 1 << 8;
    pub const RIGHT_THUMB: u16 = 1 << 9;
    pub const DPAD_UP: u16 = 1 << 10;
    pub const DPAD_DOWN: u16 = 1 << 11;
    pub const DPAD_LEFT: u16 = 1 << 12;
    pub const DPAD_RIGHT: u16 = 1 << 13;
    pub const LEFT_TRIGGER: u16 = 1 << 14;
    pub const RIGHT_TRIGGER: u16 = 1 << 15;
}

/// Normalized pad state. Axes are positive right/down. `hat_*` is -1, 0 or 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawPad {
    pub connected: bool,
    pub buttons: u16,
    pub lx: i16,
    pub ly: i16,
    pub hat_x: i8,
    pub hat_y: i8,
}

/// A backend fills up to [`MAX_PADS`] entries and returns the connected count.
pub trait Backend {
    fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize;
}

/// The digital state published to the browser. One frame is a snapshot; the
/// browser forwards `held` to the game session, which computes its own edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub held: u32,
    pub pressed: u32,
    pub released: u32,
    pub connected: bool,
    pub pads: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct AxisState {
    negative: bool,
    positive: bool,
}

impl AxisState {
    fn update(&mut self, value: i16, enter: i16, exit: i16) -> (bool, bool) {
        let value = i32::from(value);
        let enter = i32::from(enter);
        let exit = i32::from(exit);
        let negative = if value <= -enter {
            true
        } else if value >= -exit {
            false
        } else {
            self.negative
        };
        let positive = if value >= enter {
            true
        } else if value <= exit {
            false
        } else {
            self.positive
        };
        self.negative = negative;
        self.positive = positive;
        (negative, positive)
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PadMap {
    lx: AxisState,
    ly: AxisState,
}

fn map_pad(pad: RawPad, map: &mut PadMap) -> u32 {
    let mut bits = 0;
    let b = pad.buttons;
    if b & (buttons::A | buttons::X | buttons::RIGHT_TRIGGER) != 0 {
        bits |= ACTION;
    }
    if b & (buttons::B | buttons::Y | buttons::LEFT_SHOULDER | buttons::LEFT_TRIGGER) != 0 {
        bits |= SECONDARY;
    }
    if b & (buttons::START | buttons::RIGHT_SHOULDER) != 0 {
        bits |= START;
    }
    if b & buttons::BACK != 0 {
        bits |= RESET;
    }
    if b & buttons::LEFT_THUMB != 0 {
        bits |= ACTION;
    }
    if b & buttons::RIGHT_THUMB != 0 {
        bits |= SECONDARY;
    }
    if b & buttons::DPAD_LEFT != 0 || pad.hat_x < 0 {
        bits |= LEFT;
    }
    if b & buttons::DPAD_RIGHT != 0 || pad.hat_x > 0 {
        bits |= RIGHT;
    }
    if b & buttons::DPAD_UP != 0 || pad.hat_y < 0 {
        bits |= UP;
    }
    if b & buttons::DPAD_DOWN != 0 || pad.hat_y > 0 {
        bits |= DOWN;
    }
    let (left, right) = map.lx.update(pad.lx, 12_000, 8_000);
    let (up, down) = map.ly.update(pad.ly, 12_000, 8_000);
    if left {
        bits |= LEFT;
    }
    if right {
        bits |= RIGHT;
    }
    if up {
        bits |= UP;
    }
    if down {
        bits |= DOWN;
    }
    bits & INPUT_MASK
}

/// Browser-owned gamepad state. `suspend` clears held bits and latches any button
/// that was down so it must be released before it can press again after resume.
pub struct Gamepads<B: Backend = Platform> {
    backend: B,
    pads: [RawPad; MAX_PADS],
    maps: [PadMap; MAX_PADS],
    previous: u32,
    blocked: u32,
    connected: bool,
}

impl Gamepads<Platform> {
    pub fn platform() -> Self {
        Self::with_backend(Platform::new())
    }
}

impl<B: Backend> Gamepads<B> {
    pub fn with_backend(backend: B) -> Self {
        Self {
            backend,
            pads: [RawPad::default(); MAX_PADS],
            maps: [PadMap::default(); MAX_PADS],
            previous: 0,
            blocked: 0,
            connected: false,
        }
    }

    pub fn poll(&mut self) -> Frame {
        let mut raw = 0;
        let mut connected = 0u8;
        for pad in &mut self.pads {
            *pad = RawPad::default();
        }
        let _reported = self.backend.poll(&mut self.pads);
        for (i, pad) in self.pads.iter().enumerate().take(MAX_PADS) {
            if pad.connected {
                connected = connected.saturating_add(1);
                raw |= map_pad(*pad, &mut self.maps[i]);
            } else {
                self.maps[i] = PadMap::default();
            }
        }
        self.connected = connected != 0;
        // A suspended button remains blocked until the physical input is released.
        self.blocked &= raw;
        let held = raw & !self.blocked & INPUT_MASK;
        let pressed = held & !self.previous;
        let released = self.previous & !held;
        self.previous = held;
        Frame {
            held,
            pressed,
            released,
            connected: self.connected,
            pads: connected,
        }
    }

    /// Release every held pad bit immediately. Bits that were still down are
    /// latched until a fresh release, so resuming a paused game never inherits a
    /// held direction or action.
    pub fn suspend(&mut self) -> Frame {
        let released = self.previous;
        self.blocked |= self.previous;
        self.previous = 0;
        Frame {
            held: 0,
            pressed: 0,
            released,
            connected: self.connected,
            pads: 0,
        }
    }

    pub fn held(&self) -> u32 {
        self.previous
    }

    pub fn connected(&self) -> bool {
        self.connected
    }
}

impl<B: Backend> Default for Gamepads<B>
where
    B: Default,
{
    fn default() -> Self {
        Self::with_backend(B::default())
    }
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::Platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Platform;

#[cfg(not(any(windows, target_os = "linux")))]
#[derive(Default)]
pub struct Platform;

#[cfg(not(any(windows, target_os = "linux")))]
impl Platform {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
impl Backend for Platform {
    fn poll(&mut self, _: &mut [RawPad; MAX_PADS]) -> usize {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    #[derive(Default)]
    struct Fake;

    impl Backend for Fake {
        fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize {
            *pads = [RawPad::default(); MAX_PADS];
            0
        }
    }

    struct Sequence {
        frames: Vec<[RawPad; MAX_PADS]>,
    }

    impl Backend for Sequence {
        fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize {
            if self.frames.is_empty() {
                *pads = [RawPad::default(); MAX_PADS];
                return 0;
            }
            let frame = self.frames.remove(0);
            *pads = frame;
            frame.iter().filter(|pad| pad.connected).count()
        }
    }

    struct Script {
        steps: Vec<RawPad>,
    }

    impl Backend for Script {
        fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize {
            if let Some(pad) = self.steps.first().copied() {
                self.steps.remove(0);
                pads[0] = pad;
                1
            } else {
                pads[0] = RawPad::default();
                1
            }
        }
    }

    fn pad(buttons: u16) -> RawPad {
        RawPad {
            connected: true,
            buttons,
            ..RawPad::default()
        }
    }

    fn stick(lx: i16, ly: i16) -> RawPad {
        RawPad {
            connected: true,
            lx,
            ly,
            ..RawPad::default()
        }
    }

    #[test]
    fn buttons_map_to_shared_game_bits() {
        assert_eq!(map_pad(pad(buttons::A), &mut PadMap::default()), ACTION);
        assert_eq!(map_pad(pad(buttons::B), &mut PadMap::default()), SECONDARY);
        assert_eq!(map_pad(pad(buttons::START), &mut PadMap::default()), START);
        assert_eq!(map_pad(pad(buttons::BACK), &mut PadMap::default()), RESET);
        assert_eq!(map_pad(pad(buttons::DPAD_UP), &mut PadMap::default()), UP);
        assert_eq!(
            map_pad(
                pad(buttons::DPAD_LEFT | buttons::DPAD_DOWN),
                &mut PadMap::default()
            ),
            LEFT | DOWN
        );
        assert_eq!(map_pad(pad(u16::MAX), &mut PadMap::default()), 255);
        assert_eq!(map_pad(pad(0), &mut PadMap::default()), 0);
    }

    #[test]
    fn analog_axes_use_deadzone_and_hysteresis() {
        let mut map = PadMap::default();
        assert_eq!(map_pad(stick(5_000, 0), &mut map), 0);
        assert_eq!(map_pad(stick(12_000, 0), &mut map), RIGHT);
        assert_eq!(map_pad(stick(9_000, 0), &mut map), RIGHT);
        assert_eq!(map_pad(stick(7_000, 0), &mut map), 0);
        assert_eq!(map_pad(stick(-12_000, 0), &mut map), LEFT);
        assert_eq!(map_pad(stick(-9_000, 0), &mut map), LEFT);
        assert_eq!(map_pad(stick(-7_000, 0), &mut map), 0);
        assert_eq!(map_pad(stick(0, 12_000), &mut map), DOWN);
        assert_eq!(map_pad(stick(0, -12_000), &mut map), UP);
        let hat = RawPad {
            connected: true,
            hat_x: -1,
            hat_y: 1,
            ..RawPad::default()
        };
        assert_eq!(map_pad(hat, &mut map), LEFT | DOWN);
    }

    #[test]
    fn disconnect_releases_only_the_missing_pad() {
        let empty = [RawPad::default(); MAX_PADS];
        let mut both = empty;
        both[0] = pad(buttons::A);
        both[1] = pad(buttons::DPAD_LEFT);
        let mut second_pad = empty;
        second_pad[1] = pad(buttons::DPAD_LEFT);
        let mut pads = Gamepads::with_backend(Sequence {
            frames: vec![both, second_pad, empty],
        });
        let first = pads.poll();
        assert_eq!(first.held, ACTION | LEFT);
        assert_eq!(first.pads, 2);
        assert!(first.connected);
        // Pad 0 disappears, pad 1 stays.
        let second = pads.poll();
        assert_eq!(second.held, LEFT);
        assert_eq!(second.released, ACTION);
        assert_eq!(second.pads, 1);
        // Pad 1 disappears too.
        let third = pads.poll();
        assert_eq!(third.held, 0);
        assert_eq!(third.released, LEFT);
        assert!(!third.connected);
    }

    #[test]
    fn suspend_blocks_held_input_until_a_fresh_release() {
        let mut pads = Gamepads::with_backend(Script {
            steps: vec![
                pad(buttons::A | buttons::DPAD_RIGHT),
                pad(buttons::A | buttons::DPAD_RIGHT),
                pad(0),
                pad(buttons::A),
            ],
        });
        assert_eq!(pads.poll().held, ACTION | RIGHT);
        let suspended = pads.suspend();
        assert_eq!(suspended.held, 0);
        assert_eq!(suspended.released, ACTION | RIGHT);
        assert_eq!(pads.held(), 0);
        // Poll 1 after resume: still physically held; no inherited input.
        assert_eq!(pads.poll().held, 0);
        // Poll 2: released; latch clears without a press.
        let released = pads.poll();
        assert_eq!(released.held, 0);
        assert_eq!(released.pressed, 0);
        // Poll 3: a new press is a real edge.
        let pressed = pads.poll();
        assert_eq!(pressed.held, ACTION);
        assert_eq!(pressed.pressed, ACTION);
        assert_eq!(pressed.released, 0);
    }

    #[test]
    fn suspend_without_a_held_button_does_not_block_future_presses() {
        let mut pads = Gamepads::with_backend(Script {
            steps: vec![pad(0), pad(buttons::A)],
        });
        assert_eq!(pads.poll().held, 0);
        pads.suspend();
        let pressed = pads.poll();
        assert_eq!(pressed.held, ACTION);
        assert_eq!(pressed.pressed, ACTION);
    }

    #[test]
    fn four_pads_combine_and_the_slot_count_is_bounded() {
        let mut all = [RawPad::default(); MAX_PADS];
        all[0] = pad(buttons::A);
        all[1] = pad(buttons::DPAD_LEFT);
        all[2] = pad(buttons::DPAD_RIGHT);
        all[3] = pad(buttons::START);
        let mut pads = Gamepads::with_backend(Sequence { frames: vec![all] });
        let frame = pads.poll();
        assert_eq!(frame.held, ACTION | LEFT | RIGHT | START);
        assert_eq!(frame.pads, MAX_PADS as u8);
        assert_eq!(MAX_PADS, 4);
    }

    #[test]
    fn zero_pads_and_unknown_buttons_fail_silently() {
        let mut pads = Gamepads::with_backend(Fake);
        assert_eq!(pads.poll(), Frame::default());
        assert_eq!(map_pad(pad(0), &mut PadMap::default()), 0);
        assert_eq!(INPUT_MASK, 1023);
    }
}
