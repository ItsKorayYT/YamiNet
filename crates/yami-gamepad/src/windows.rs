//! Windows XInput backend. The only unsafe code in this crate: a bounded call
//! to the documented XInput API, with no pointers escaping this module.
#![allow(unsafe_code)]

use super::{buttons, Backend, RawPad, MAX_PADS};
use windows_sys::Win32::UI::Input::XboxController::{
    XInputGetState, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK,
    XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT,
    XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
    XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START,
    XINPUT_GAMEPAD_TRIGGER_THRESHOLD, XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE,
};

#[derive(Default)]
pub struct Platform;

impl Platform {
    pub fn new() -> Self {
        Self
    }
}

impl Backend for Platform {
    fn poll(&mut self, pads: &mut [RawPad; MAX_PADS]) -> usize {
        let mut connected = 0;
        for (index, pad) in pads.iter_mut().enumerate() {
            let mut state = XINPUT_STATE::default();
            // SAFETY: `state` is a valid, initialized, stack-local XINPUT_STATE
            // for the duration of the call; the slot index is 0..MAX_PADS and the
            // API has no other pointer arguments or retained references.
            let result = unsafe { XInputGetState(index as u32, &mut state) };
            if result != 0 {
                continue;
            }
            let gamepad = state.Gamepad;
            let mut mask = 0;
            let source = gamepad.wButtons;
            for (native, abstract_bit) in [
                (XINPUT_GAMEPAD_A, buttons::A),
                (XINPUT_GAMEPAD_B, buttons::B),
                (XINPUT_GAMEPAD_X, buttons::X),
                (XINPUT_GAMEPAD_Y, buttons::Y),
                (XINPUT_GAMEPAD_LEFT_SHOULDER, buttons::LEFT_SHOULDER),
                (XINPUT_GAMEPAD_RIGHT_SHOULDER, buttons::RIGHT_SHOULDER),
                (XINPUT_GAMEPAD_BACK, buttons::BACK),
                (XINPUT_GAMEPAD_START, buttons::START),
                (XINPUT_GAMEPAD_LEFT_THUMB, buttons::LEFT_THUMB),
                (XINPUT_GAMEPAD_RIGHT_THUMB, buttons::RIGHT_THUMB),
                (XINPUT_GAMEPAD_DPAD_UP, buttons::DPAD_UP),
                (XINPUT_GAMEPAD_DPAD_DOWN, buttons::DPAD_DOWN),
                (XINPUT_GAMEPAD_DPAD_LEFT, buttons::DPAD_LEFT),
                (XINPUT_GAMEPAD_DPAD_RIGHT, buttons::DPAD_RIGHT),
            ] {
                if source & native != 0 {
                    mask |= abstract_bit;
                }
            }
            if gamepad.bLeftTrigger > XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u8 {
                mask |= buttons::LEFT_TRIGGER;
            }
            if gamepad.bRightTrigger > XINPUT_GAMEPAD_TRIGGER_THRESHOLD as u8 {
                mask |= buttons::RIGHT_TRIGGER;
            }
            *pad = RawPad {
                connected: true,
                buttons: mask,
                lx: gamepad.sThumbLX,
                // XInput reports positive Y upward; the shared convention is
                // positive downward. Negating i16::MIN stays i16::MIN.
                ly: gamepad.sThumbLY.saturating_neg(),
                hat_x: 0,
                hat_y: 0,
            };
            connected += 1;
        }
        connected
    }
}
