//! Ready-made views over the host's digital input mask.
//!
//! Native controllers are mapped to the same held/pressed/released bits as the
//! keyboard, so games do not need a controller-specific host import. The browser
//! applies dead zones, supports multiple pads and clears held bits on focus loss,
//! disconnect or pause; these helpers only interpret the resulting mask.

use crate::v2::Input;
use crate::{DOWN, LEFT, RIGHT, START, UP};

/// Horizontal digital axis: -1 left, 0 neutral, 1 right.
pub fn x(input: &Input) -> i8 {
    input.down(RIGHT) as i8 - input.down(LEFT) as i8
}

/// Vertical digital axis: -1 up, 0 neutral, 1 down.
pub fn y(input: &Input) -> i8 {
    input.down(DOWN) as i8 - input.down(UP) as i8
}

/// Combined movement vector. Diagonals stay digital; this keeps tile movement
/// predictable and avoids normalisation that would require a square root.
pub fn direction(input: &Input) -> (i8, i8) {
    (x(input), y(input))
}

pub fn pressed_any(input: &Input, mask: u32) -> bool {
    input.pressed & mask != 0
}

pub fn down_any(input: &Input, mask: u32) -> bool {
    input.held & mask != 0
}

pub fn released_any(input: &Input, mask: u32) -> bool {
    input.released & mask != 0
}

/// True when a start/action button was pressed this frame. A controller's A
/// button and the keyboard's Space/Enter both map to `START`/`ACTION`.
pub fn confirm(input: &Input) -> bool {
    pressed_any(
        input,
        START | crate::ACTION | crate::SECONDARY | crate::MOUSE_LEFT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_and_edges_follow_the_shared_mask() {
        let mut input = Input::default();
        assert_eq!(direction(&input), (0, 0));
        input.held = LEFT | UP;
        input.pressed = RIGHT;
        input.released = DOWN;
        assert_eq!(x(&input), -1);
        assert_eq!(y(&input), -1);
        assert_eq!(direction(&input), (-1, -1));
        assert!(down_any(&input, LEFT | RIGHT));
        assert!(pressed_any(&input, RIGHT));
        assert!(!pressed_any(&input, LEFT));
        assert!(released_any(&input, DOWN));
        assert!(confirm(&Input {
            pressed: crate::ACTION,
            ..Input::default()
        }));
        assert!(!confirm(&Input {
            held: crate::ACTION,
            ..Input::default()
        }));
    }
}
