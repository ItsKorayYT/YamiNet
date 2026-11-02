//! Bounded tilemap queries, visible-tile iteration and sprite drawing.
//!
//! Tile `0` means empty; a non-zero tile is `sheet frame + 1`. The map borrows a
//! caller-owned slice so it can live in static memory or on the guest stack. No
//! allocation, decoding or host access happens here beyond the sprite commands a
//! caller explicitly draws.

use crate::sheet::Sheet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileMap<'a> {
    pub tiles: &'a [u16],
    pub width: u16,
    pub height: u16,
    pub tile_w: u16,
    pub tile_h: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleTile {
    /// Non-zero tile value (`0` is never yielded).
    pub tile: u16,
    pub column: u16,
    pub row: u16,
    /// Screen coordinates relative to the camera's top-left corner.
    pub x: i32,
    pub y: i32,
}

fn floor_div(value: i32, divisor: i32) -> i32 {
    debug_assert!(divisor > 0);
    (value as i64).div_euclid(divisor as i64) as i32
}

fn ceil_div(value: i32, divisor: i32) -> i32 {
    debug_assert!(divisor > 0);
    let value = value as i64;
    let divisor = divisor as i64;
    (value + divisor - 1).div_euclid(divisor) as i32
}

fn floor_f32(value: f32) -> i32 {
    let truncated = value as i32;
    if (truncated as f32) > value {
        truncated - 1
    } else {
        truncated
    }
}

impl<'a> TileMap<'a> {
    /// Construct without panicking. An inconsistent slice length is treated as
    /// a truncated map: missing cells read as empty.
    pub const fn new(tiles: &'a [u16], width: u16, height: u16, tile_w: u16, tile_h: u16) -> Self {
        Self {
            tiles,
            width,
            height,
            tile_w,
            tile_h,
        }
    }

    pub const fn is_usable(self) -> bool {
        self.tile_w != 0 && self.tile_h != 0 && self.width != 0 && self.height != 0
    }

    pub fn tile(self, column: u16, row: u16) -> u16 {
        if column >= self.width || row >= self.height {
            return 0;
        }
        let index = row as usize * self.width as usize + column as usize;
        self.tiles.get(index).copied().unwrap_or(0)
    }

    /// Non-zero tiles at or above `first_solid` block movement.
    pub fn world_width(self) -> i32 {
        (self.width as i32).saturating_mul(self.tile_w as i32)
    }

    pub fn world_height(self) -> i32 {
        (self.height as i32).saturating_mul(self.tile_h as i32)
    }

    /// Non-zero tiles at or above `first_solid` block movement.
    pub fn solid_tile(self, tile: u16, first_solid: u16) -> bool {
        tile != 0 && tile >= first_solid
    }

    pub fn solid_at(self, column: u16, row: u16, first_solid: u16) -> bool {
        self.solid_tile(self.tile(column, row), first_solid)
    }

    pub fn point_solid(self, x: i32, y: i32, first_solid: u16) -> bool {
        if !self.is_usable() {
            return false;
        }
        let column = floor_div(x, self.tile_w as i32);
        let row = floor_div(y, self.tile_h as i32);
        if column < 0 || row < 0 || column >= self.width as i32 || row >= self.height as i32 {
            // The world edge is solid so actors cannot walk out of the level.
            return true;
        }
        self.solid_at(column as u16, row as u16, first_solid)
    }

    /// True when an axis-aligned rectangle touches any solid tile or the world
    /// edge. The right and bottom edges are exclusive.
    pub fn rect_solid(self, x: f32, y: f32, w: f32, h: f32, first_solid: u16) -> bool {
        if !self.is_usable()
            || !x.is_finite()
            || !y.is_finite()
            || !w.is_finite()
            || !h.is_finite()
            || w <= 0.0
            || h <= 0.0
        {
            return false;
        }
        let right_edge = x + w;
        let bottom_edge = y + h;
        if !right_edge.is_finite() || !bottom_edge.is_finite() {
            return false;
        }
        let left = floor_f32(x / self.tile_w as f32);
        let right = floor_f32((right_edge - 0.001) / self.tile_w as f32);
        let top = floor_f32(y / self.tile_h as f32);
        let bottom = floor_f32((bottom_edge - 0.001) / self.tile_h as f32);
        if left < 0 || top < 0 || right >= self.width as i32 || bottom >= self.height as i32 {
            return true;
        }
        for row in top..=bottom {
            for column in left..=right {
                if self.solid_at(column as u16, row as u16, first_solid) {
                    return true;
                }
            }
        }
        false
    }

    /// Iterate only non-empty tiles intersecting the view rectangle.
    pub fn visible(self, camera_x: i32, camera_y: i32, view_w: i32, view_h: i32) -> Visible<'a> {
        if !self.is_usable() || view_w <= 0 || view_h <= 0 {
            return Visible {
                map: self,
                column: 0,
                row: 0,
                end_column: 0,
                end_row: 0,
                camera_x,
                camera_y,
                done: true,
            };
        }
        let start_column =
            floor_div(camera_x, self.tile_w as i32).clamp(0, self.width as i32) as u16;
        let start_row = floor_div(camera_y, self.tile_h as i32).clamp(0, self.height as i32) as u16;
        let end_column = ceil_div(camera_x.saturating_add(view_w), self.tile_w as i32)
            .clamp(0, self.width as i32) as u16;
        let end_row = ceil_div(camera_y.saturating_add(view_h), self.tile_h as i32)
            .clamp(0, self.height as i32) as u16;
        let done = start_column >= end_column || start_row >= end_row;
        Visible {
            map: self,
            column: start_column,
            row: start_row,
            end_column,
            end_row,
            camera_x,
            camera_y,
            done,
        }
    }

    /// Draw every visible non-empty tile as one sprite command, stopping after
    /// `max_commands`. Returns the number of commands emitted. Pass the host's
    /// remaining command budget (512 minus HUD/effects) to stay within limits.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        self,
        sheet: Sheet,
        camera_x: i32,
        camera_y: i32,
        view_w: i32,
        view_h: i32,
        layer: i32,
        max_commands: usize,
    ) -> usize {
        let mut count = 0;
        for tile in self.visible(camera_x, camera_y, view_w, view_h) {
            if count >= max_commands {
                break;
            }
            if sheet.draw(tile.tile as u32 - 1, tile.x, tile.y, layer) {
                count += 1;
            }
        }
        count
    }
}

pub struct Visible<'a> {
    map: TileMap<'a>,
    column: u16,
    row: u16,
    end_column: u16,
    end_row: u16,
    camera_x: i32,
    camera_y: i32,
    done: bool,
}

impl Iterator for Visible<'_> {
    type Item = VisibleTile;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || !self.map.is_usable() {
            return None;
        }
        loop {
            if self.column >= self.end_column {
                self.column = 0;
                self.row = self.row.saturating_add(1);
                if self.row >= self.end_row {
                    self.done = true;
                    return None;
                }
            }
            let column = self.column;
            let row = self.row;
            self.column += 1;
            let tile = self.map.tile(column, row);
            if tile != 0 {
                return Some(VisibleTile {
                    tile,
                    column,
                    row,
                    x: (column as i32)
                        .saturating_mul(self.map.tile_w as i32)
                        .saturating_sub(self.camera_x),
                    y: (row as i32)
                        .saturating_mul(self.map.tile_h as i32)
                        .saturating_sub(self.camera_y),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec::Vec;

    fn map() -> TileMap<'static> {
        // 4x3: 0 empty, 1 solid, 2 solid/decorative.
        static TILES: [u16; 12] = [1, 0, 0, 2, 0, 1, 0, 0, 0, 0, 2, 0];
        TileMap::new(&TILES, 4, 3, 16, 16)
    }

    #[test]
    fn tile_access_is_bounds_checked_and_solidity_is_thresholded() {
        let map = map();
        assert_eq!(map.tile(0, 0), 1);
        assert_eq!(map.tile(3, 2), 0);
        assert_eq!(map.tile(99, 0), 0);
        assert_eq!(map.world_width(), 64);
        assert!(map.solid_at(0, 0, 1));
        assert!(!map.solid_at(1, 0, 1));
        assert!(map.solid_at(2, 2, 2));
        assert!(!map.solid_at(2, 2, 3));
        assert!(map.point_solid(0, 0, 1));
        assert!(!map.point_solid(20, 4, 1));
        assert!(map.point_solid(-1, 0, 1));
        assert!(map.point_solid(64, 0, 1));
        assert!(map.rect_solid(0.0, 0.0, 8.0, 8.0, 1));
        assert!(!map.rect_solid(48.0, 40.0, 8.0, 8.0, 1));
        assert!(map.rect_solid(-2.0, 20.0, 4.0, 8.0, 1));
        // Huge finite rectangles hit the world edge immediately instead of
        // iterating a saturating i32 range.
        assert!(map.rect_solid(0.0, 0.0, 1.0e30, 1.0e30, 1));
        assert!(!map.rect_solid(f32::INFINITY, 0.0, 8.0, 8.0, 1));
    }

    #[test]
    fn visible_iterator_culls_to_camera_and_skips_empty_tiles() {
        let visible: Vec<_> = map().visible(0, 0, 32, 32).collect();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].tile, 1);
        assert_eq!((visible[0].x, visible[0].y), (0, 0));
        assert_eq!(visible[1].tile, 1);
        assert_eq!((visible[1].x, visible[1].y), (16, 16));
        let visible: Vec<_> = map().visible(0, 0, 16, 16).collect();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].column, 0);
        // Camera beyond the world yields no tiles.
        assert_eq!(map().visible(1000, 0, 64, 64).count(), 0);
        assert_eq!(map().visible(0, 0, 0, 32).count(), 0);
        let degenerate = TileMap::new(&[], 4, 3, 0, 16);
        assert_eq!(degenerate.visible(0, 0, 32, 32).count(), 0);
    }

    #[test]
    fn truncated_slices_read_as_empty_and_edge_is_solid() {
        static TILES: [u16; 2] = [1, 1];
        let map = TileMap::new(&TILES, 4, 4, 16, 16);
        assert_eq!(map.tile(1, 0), 1);
        assert_eq!(map.tile(2, 0), 0);
        assert!(map.point_solid(-1, -1, 1));
        assert!(map.point_solid(64, 64, 1));
    }
}
