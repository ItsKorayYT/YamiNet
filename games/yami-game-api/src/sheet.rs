//! Sprite-sheet geometry and drawing helpers.
//!
//! Sheets are declared by the game (the host only sees declared PNG resources),
//! so all arithmetic here is checked and bounded. A sheet that does not fit in
//! the host's 2048-pixel texture limit simply fails to resolve frames instead of
//! sending invalid commands to the sandbox.

use crate::v2::Sprite;

/// A regular grid of equally sized cells inside one PNG asset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub asset: u32,
    pub frame_w: u16,
    pub frame_h: u16,
    pub columns: u16,
    pub rows: u16,
    pub margin: u16,
    pub spacing: u16,
}

impl Sheet {
    pub const fn new(asset: u32, frame_w: u16, frame_h: u16, columns: u16, rows: u16) -> Self {
        Self {
            asset,
            frame_w,
            frame_h,
            columns,
            rows,
            margin: 0,
            spacing: 0,
        }
    }

    /// A sheet with outer padding and gaps between cells.
    pub const fn padded(
        asset: u32,
        frame_w: u16,
        frame_h: u16,
        columns: u16,
        rows: u16,
        margin: u16,
        spacing: u16,
    ) -> Self {
        Self {
            asset,
            frame_w,
            frame_h,
            columns,
            rows,
            margin,
            spacing,
        }
    }

    pub fn frame_count(self) -> u32 {
        self.columns as u32 * self.rows as u32
    }

    pub fn is_empty(self) -> bool {
        self.frame_count() == 0 || self.frame_w == 0 || self.frame_h == 0
    }

    /// Source rectangle `[x, y, w, h]` for a linear frame index (row-major).
    pub fn source(self, frame: u32) -> Option<[i32; 4]> {
        if self.is_empty() || frame >= self.frame_count() {
            return None;
        }
        let column = frame % self.columns as u32;
        let row = frame / self.columns as u32;
        let stride_x = (self.frame_w as u32).checked_add(self.spacing as u32)?;
        let stride_y = (self.frame_h as u32).checked_add(self.spacing as u32)?;
        let x = (self.margin as u32).checked_add(column.checked_mul(stride_x)?)?;
        let y = (self.margin as u32).checked_add(row.checked_mul(stride_y)?)?;
        // The source must stay within the validated PNG dimensions. 2048 is the
        // host's decoded texture bound, so reject impossible sheets early.
        if x.checked_add(self.frame_w as u32)? > 2048 || y.checked_add(self.frame_h as u32)? > 2048
        {
            return None;
        }
        Some([x as i32, y as i32, self.frame_w as i32, self.frame_h as i32])
    }

    pub fn cell(self, column: u16, row: u16) -> Option<[i32; 4]> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.source(row as u32 * self.columns as u32 + column as u32)
    }

    /// Draw a frame at its natural size. Returns false for an invalid frame.
    pub fn draw(self, frame: u32, x: i32, y: i32, layer: i32) -> bool {
        let Some(source) = self.source(frame) else {
            return false;
        };
        Sprite {
            asset: self.asset,
            source,
            size: [self.frame_w as i32, self.frame_h as i32],
            layer,
        }
        .draw(x, y);
        true
    }

    /// Draw a frame stretched into `w × h` destination pixels.
    pub fn draw_scaled(self, frame: u32, x: i32, y: i32, w: i32, h: i32, layer: i32) -> bool {
        let Some(source) = self.source(frame) else {
            return false;
        };
        if w <= 0 || h <= 0 {
            return false;
        }
        Sprite {
            asset: self.asset,
            source,
            size: [w, h],
            layer,
        }
        .draw(x, y);
        true
    }

    /// Draw a full animation row by absolute frame. The host must still be
    /// called from `update`; this helper is only a thin geometry adapter.
    pub fn draw_frame(self, frame: u32, x: i32, y: i32, layer: i32) -> bool {
        self.draw(frame, x, y, layer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_resolve_row_major_with_padding_and_spacing() {
        let sheet = Sheet::new(0, 16, 24, 4, 2);
        assert_eq!(sheet.frame_count(), 8);
        assert_eq!(sheet.source(0), Some([0, 0, 16, 24]));
        assert_eq!(sheet.source(3), Some([48, 0, 16, 24]));
        assert_eq!(sheet.source(4), Some([0, 24, 16, 24]));
        assert_eq!(sheet.source(8), None);
        assert_eq!(sheet.cell(2, 1), Some([32, 24, 16, 24]));
        assert_eq!(sheet.cell(4, 0), None);
        let padded = Sheet::padded(1, 10, 10, 2, 2, 3, 2);
        assert_eq!(padded.source(0), Some([3, 3, 10, 10]));
        assert_eq!(padded.source(1), Some([15, 3, 10, 10]));
        assert_eq!(padded.source(2), Some([3, 15, 10, 10]));
    }

    #[test]
    fn invalid_sheets_fail_closed_without_overflow() {
        for sheet in [
            Sheet::new(0, 0, 8, 2, 2),
            Sheet::new(0, 8, 0, 2, 2),
            Sheet::new(0, 8, 8, 0, 2),
            Sheet::new(0, 8, 8, 2, 0),
        ] {
            assert!(sheet.is_empty());
            assert_eq!(sheet.source(0), None);
        }
        let huge = Sheet::padded(0, u16::MAX, u16::MAX, 2, 2, u16::MAX, u16::MAX);
        assert_eq!(huge.source(0), None);
        assert_eq!(huge.source(3), None);
    }
}
