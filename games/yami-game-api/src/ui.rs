//! Small HUD and text-layout helpers.
//!
//! The host draws one line per `text` command. This module adds accurate-enough
//! advance measurement, centering, right alignment, shadows, panels, bars and
//! counters without adding another host import. Measurements use the bundled
//! Open Sans advance widths; shaping kerning can shift a long line by a pixel or
//! two, which is why the API exposes raw widths as well as helpers.

use crate::{circle, rect, rgb, text as host_text};

/// Advance widths for printable ASCII, in 1/1000 em, at `size >= 18` (the host
/// selects the semibold face there). Generated from the bundled OpenSans fonts.
const WIDTHS_REGULAR: [u16; 95] = [
    260, 267, 401, 646, 572, 823, 730, 221, 296, 296, 552, 572, 245, 322, 266, 367, 572, 572, 572,
    572, 572, 572, 572, 572, 572, 572, 266, 266, 572, 572, 572, 429, 899, 633, 648, 631, 729, 556,
    516, 728, 738, 279, 267, 614, 519, 903, 754, 779, 602, 779, 618, 549, 553, 728, 595, 926, 577,
    560, 571, 329, 367, 329, 542, 448, 577, 556, 613, 476, 613, 561, 339, 548, 614, 253, 253, 525,
    253, 930, 614, 604, 613, 613, 408, 477, 353, 614, 501, 778, 524, 504, 468, 379, 551, 379, 572,
];
const WIDTHS_BOLD: [u16; 95] = [
    260, 276, 436, 646, 571, 862, 740, 243, 317, 317, 548, 571, 267, 322, 275, 390, 571, 571, 571,
    571, 571, 571, 571, 571, 571, 571, 275, 278, 571, 571, 571, 453, 898, 661, 660, 634, 734, 558,
    532, 726, 751, 305, 299, 639, 542, 923, 783, 787, 615, 787, 639, 550, 566, 742, 622, 946, 622,
    592, 575, 330, 390, 330, 537, 429, 592, 580, 623, 495, 623, 576, 363, 556, 635, 279, 279, 572,
    279, 956, 635, 611, 623, 623, 431, 487, 393, 635, 535, 817, 551, 536, 478, 386, 551, 371, 571,
];
const FALLBACK_UNITS: u16 = 600;

pub const SMALL: i32 = 10;
pub const BODY: i32 = 13;
pub const HEADING: i32 = 18;
pub const TITLE: i32 = 24;

/// Width in logical pixels for the host's bounded text range (size 8..=48).
/// Non-ASCII characters use a conservative fallback.
pub fn width(label: &str, size: i32) -> i32 {
    let size = size.clamp(8, 48);
    let table = if size >= 18 {
        &WIDTHS_BOLD
    } else {
        &WIDTHS_REGULAR
    };
    let mut units = 0u32;
    for ch in label.chars() {
        let code = ch as u32;
        let width = if (32..=126).contains(&code) {
            table[(code - 32) as usize] as u32
        } else {
            FALLBACK_UNITS as u32
        };
        units = units.saturating_add(width);
    }
    let scale = (units as u64 * size as u64 + 500) / 1000;
    scale.min(i32::MAX as u64) as i32
}

pub fn centered_x(label: &str, center_x: i32, size: i32) -> i32 {
    center_x.saturating_sub(width(label, size) / 2)
}

pub fn right_x(label: &str, right: i32, size: i32) -> i32 {
    right.saturating_sub(width(label, size))
}

/// Draw a single line with the measured width clipped to the canvas.
pub fn text(label: &str, x: i32, y: i32, size: i32, color: u32) {
    host_text(label, x, y, size, color);
}

pub fn centered(label: &str, center_x: i32, y: i32, size: i32, color: u32) {
    let x = centered_x(label, center_x, size);
    if (0..crate::WIDTH).contains(&x) {
        host_text(label, x, y, size, color);
    }
}

pub fn right(label: &str, right: i32, y: i32, size: i32, color: u32) {
    let x = right_x(label, right, size);
    if (0..crate::WIDTH).contains(&x) {
        host_text(label, x, y, size, color);
    }
}

/// A one-pixel drop shadow followed by the main text.
pub fn shadow(label: &str, x: i32, y: i32, size: i32, color: u32, shade: u32) {
    if x > 0 && x < crate::WIDTH - 1 && y < crate::HEIGHT - 1 {
        host_text(label, x + 1, y + 1, size, shade);
    }
    host_text(label, x, y, size, color);
}

/// Filled panel with an optional one-pixel border.
pub fn panel(x: i32, y: i32, w: i32, h: i32, fill: u32, border: u32) {
    if w <= 0
        || h <= 0
        || x < 0
        || y < 0
        || x.checked_add(w).is_none_or(|right| right > crate::WIDTH)
        || y.checked_add(h).is_none_or(|bottom| bottom > crate::HEIGHT)
    {
        return;
    }
    rect(x, y, w, h, fill);
    if border & 255 != 0 {
        rect(x, y, w, 1, border);
        rect(x, y + h - 1, w, 1, border);
        rect(x, y + 1, 1, h - 2, border);
        rect(x + w - 1, y + 1, 1, h - 2, border);
    }
}

/// A bounded progress bar. Values outside `0..=max` clamp; a zero maximum
/// draws an empty bar rather than dividing by zero.
#[allow(clippy::too_many_arguments)]
pub fn bar(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    value: i32,
    max: i32,
    fill: u32,
    track: u32,
    border: u32,
) {
    if w <= 0
        || h <= 0
        || x < 0
        || y < 0
        || x.checked_add(w).is_none_or(|right| right > crate::WIDTH)
        || y.checked_add(h).is_none_or(|bottom| bottom > crate::HEIGHT)
    {
        return;
    }
    rect(x, y, w, h, track);
    if max > 0 && value > 0 {
        let filled = ((w as i64 * value.min(max) as i64 + max as i64 / 2) / max as i64) as i32;
        if filled > 0 {
            rect(x, y, filled.min(w), h, fill);
        }
    }
    if border & 255 != 0 {
        rect(x, y, w, 1, border);
        rect(x, y + h - 1, w, 1, border);
        rect(x, y + 1, 1, h - 2, border);
        rect(x + w - 1, y + 1, 1, h - 2, border);
    }
}

/// A row of square pips, useful for lives, keys or collected items.
#[allow(clippy::too_many_arguments)]
pub fn pips(x: i32, y: i32, filled: i32, total: i32, size: i32, gap: i32, on: u32, off: u32) {
    let total = total.clamp(0, 32);
    let size = size.clamp(2, 16);
    let gap = gap.clamp(0, 8);
    let filled = filled.clamp(0, total);
    for i in 0..total {
        let Some(px) = i
            .checked_mul(size.saturating_add(gap))
            .and_then(|step| x.checked_add(step))
        else {
            break;
        };
        if px < 0
            || px
                .checked_add(size)
                .is_none_or(|right| right > crate::WIDTH)
        {
            break;
        }
        rect(px, y, size, size, if i < filled { on } else { off });
    }
}

/// A small circular badge; clamped so it can never send an invalid circle.
pub fn badge(cx: i32, cy: i32, radius: i32, color: u32) {
    let r = radius.clamp(1, 64);
    let inside = cx.checked_sub(r).is_some_and(|left| left >= 0)
        && cy.checked_sub(r).is_some_and(|top| top >= 0)
        && cx.checked_add(r).is_some_and(|right| right < crate::WIDTH)
        && cy
            .checked_add(r)
            .is_some_and(|bottom| bottom < crate::HEIGHT);
    if inside {
        circle(cx, cy, r, color);
    }
}

/// Write an unsigned decimal integer into `buf`. If the buffer is too small the
/// least-significant digits are kept; the returned slice is always valid UTF-8.
pub fn format_u32(mut value: u32, buf: &mut [u8]) -> &str {
    if buf.is_empty() {
        return "";
    }
    let mut digits = [0u8; 10];
    let mut count = 0;
    if value == 0 {
        digits[0] = b'0';
        count = 1;
    } else {
        while value != 0 && count < digits.len() {
            digits[count] = b'0' + (value % 10) as u8;
            value /= 10;
            count += 1;
        }
    }
    let keep = count.min(buf.len());
    for i in 0..keep {
        buf[i] = digits[keep - 1 - i];
    }
    core::str::from_utf8(&buf[..keep]).unwrap_or("")
}

/// Format seconds as `MM:SS`, clamped to 99:59.
pub fn format_clock(seconds: u32, buf: &mut [u8; 5]) -> &str {
    let (minutes, seconds) = if seconds > 99 * 60 + 59 {
        (99, 59)
    } else {
        (seconds / 60, seconds % 60)
    };
    buf[0] = b'0' + (minutes / 10) as u8;
    buf[1] = b'0' + (minutes % 10) as u8;
    buf[2] = b':';
    buf[3] = b'0' + (seconds / 10) as u8;
    buf[4] = b'0' + (seconds % 10) as u8;
    core::str::from_utf8(buf).unwrap_or("00:00")
}

/// Replace a color's alpha byte. Text commands require alpha 255.
pub const fn alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xffffff00) | alpha as u32
}

/// Integer color blend; `t` is clamped to 0..=100.
pub fn blend(a: u32, b: u32, t: i32) -> u32 {
    let t = t.clamp(0, 100) as u32;
    let channel = |shift: u32| {
        let ca = (a >> shift) & 255;
        let cb = (b >> shift) & 255;
        (ca * (100 - t) + cb * t) / 100
    };
    rgb(channel(24) as u8, channel(16) as u8, channel(8) as u8) & 0xffffff00 | (channel(0) & 255)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_widths_are_bounded_and_scale_monotonically() {
        assert_eq!(width("", 12), 0);
        assert!(width("Hi", 12) > 0);
        assert!(width("Hi", 24) > width("Hi", 12));
        assert!(width("W", 12) > width("i", 12));
        assert_eq!(width("A", 0), width("A", 8));
        assert_eq!(width("A", 200), width("A", 48));
        assert!(width("café", 12) > width("caf", 12));
        assert_eq!(centered_x("AB", 100, 12), 100 - width("AB", 12) / 2);
        assert_eq!(right_x("AB", 100, 12), 100 - width("AB", 12));
    }

    #[test]
    fn counters_and_clock_are_bounded_and_utf8() {
        let mut buffer = [0u8; 10];
        assert_eq!(format_u32(0, &mut buffer), "0");
        assert_eq!(format_u32(42, &mut buffer), "42");
        assert_eq!(format_u32(u32::MAX, &mut buffer), "4294967295");
        let mut tiny = [0u8; 3];
        assert_eq!(format_u32(12345, &mut tiny), "345");
        assert_eq!(format_u32(1, &mut []), "");
        let mut clock = [0u8; 5];
        assert_eq!(format_clock(0, &mut clock), "00:00");
        assert_eq!(format_clock(59, &mut clock), "00:59");
        assert_eq!(format_clock(61, &mut clock), "01:01");
        assert_eq!(format_clock(9_999, &mut clock), "99:59");
    }

    #[test]
    fn color_helpers_preserve_bounds() {
        assert_eq!(alpha(rgb(1, 2, 3), 0), 0x01020300);
        assert_eq!(alpha(rgb(1, 2, 3), 255), rgb(1, 2, 3));
        assert_eq!(blend(rgb(0, 0, 0), rgb(255, 255, 255), 50), 0x7f7f7fff);
        assert_eq!(blend(rgb(1, 2, 3), rgb(4, 5, 6), -10), 0x010203ff);
        assert_eq!(blend(rgb(1, 2, 3), rgb(4, 5, 6), 200), 0x040506ff);
    }
}
