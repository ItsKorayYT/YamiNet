//! Kinematic bodies with axis-separated tile collision.
//!
//! This is deliberately small: games keep their own player/enemy state and use
//! [`Body`] for the common AABB movement problem. Collision is resolved on each
//! axis, with substeps no larger than half a tile so fast actors cannot tunnel.

use crate::collision::Rect;
use crate::tilemap::TileMap;

pub trait TileWorld {
    fn tile_size(&self) -> (f32, f32);
    fn solid(&self, column: i32, row: i32) -> bool;
    fn tiles(&self) -> (i32, i32);
}

/// A tilemap plus the first tile value that blocks movement.
#[derive(Clone, Copy, Debug)]
pub struct SolidMap<'a> {
    pub map: TileMap<'a>,
    pub first_solid: u16,
}

impl<'a> SolidMap<'a> {
    pub const fn new(map: TileMap<'a>, first_solid: u16) -> Self {
        Self { map, first_solid }
    }
}

impl TileWorld for SolidMap<'_> {
    fn tile_size(&self) -> (f32, f32) {
        (self.map.tile_w as f32, self.map.tile_h as f32)
    }

    fn solid(&self, column: i32, row: i32) -> bool {
        if column < 0 || row < 0 || column >= self.map.width as i32 || row >= self.map.height as i32
        {
            return true;
        }
        self.map
            .solid_at(column as u16, row as u16, self.first_solid)
    }

    fn tiles(&self) -> (i32, i32) {
        (self.map.width as i32, self.map.height as i32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveResult {
    pub hit_x: bool,
    pub hit_y: bool,
    pub on_ground: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub vx: f32,
    pub vy: f32,
    pub on_ground: bool,
}

impl Body {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            vx: 0.0,
            vy: 0.0,
            on_ground: false,
        }
    }

    pub fn rect(self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    pub fn center(self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn apply_gravity(&mut self, dt: f32, gravity: f32, max_fall: f32) {
        if !dt.is_finite() || !gravity.is_finite() {
            return;
        }
        let max_fall = if max_fall.is_finite() {
            max_fall.abs()
        } else {
            f32::MAX
        };
        self.vy = (self.vy + gravity * dt.max(0.0)).clamp(-max_fall, max_fall);
    }

    /// Jump only from the ground, so buffered presses cannot double-jump.
    pub fn jump(&mut self, impulse: f32) -> bool {
        if !self.on_ground || !impulse.is_finite() || impulse <= 0.0 {
            return false;
        }
        self.vy = -impulse;
        self.on_ground = false;
        true
    }

    /// Keep the body inside the map even if a caller teleports it.
    pub fn clamp_to_world(&mut self, world: &impl TileWorld) {
        let (tile_w, tile_h) = world.tile_size();
        let (columns, rows) = world.tiles();
        if tile_w <= 0.0 || tile_h <= 0.0 || columns <= 0 || rows <= 0 {
            return;
        }
        let max_x = columns as f32 * tile_w - self.w;
        let max_y = rows as f32 * tile_h - self.h;
        if self.x < 0.0 {
            self.x = 0.0;
            self.vx = self.vx.max(0.0);
        } else if self.x > max_x {
            self.x = max_x.max(0.0);
            self.vx = self.vx.min(0.0);
        }
        if self.y < 0.0 {
            self.y = 0.0;
            self.vy = self.vy.max(0.0);
        } else if self.y > max_y {
            self.y = max_y.max(0.0);
            self.vy = self.vy.min(0.0);
            self.on_ground = true;
        }
    }
}

fn floor_i32(value: f32) -> i32 {
    let truncated = value as i32;
    if (truncated as f32) > value {
        truncated - 1
    } else {
        truncated
    }
}

fn tile_floor(value: f32, size: f32) -> i32 {
    floor_i32(value / size)
}

fn columns(body: &Body, tile_w: f32) -> (i32, i32) {
    let left = tile_floor(body.x, tile_w);
    let right = tile_floor(body.x + body.w - 0.001, tile_w);
    (left, right)
}

fn rows(body: &Body, tile_h: f32) -> (i32, i32) {
    let top = tile_floor(body.y, tile_h);
    let bottom = tile_floor(body.y + body.h - 0.001, tile_h);
    (top, bottom)
}

/// Move on X, stopping at solid tiles. Returns true when a collision occurred.
pub fn move_x(body: &mut Body, dx: f32, world: &impl TileWorld) -> bool {
    let (tile_w, _) = world.tile_size();
    if !dx.is_finite() || dx == 0.0 || tile_w <= 0.0 {
        return false;
    }
    let limit = (tile_w * 0.5).max(0.01);
    let mut remaining = dx;
    let mut hit = false;
    for _ in 0..256 {
        if remaining.abs() <= 0.0001 {
            break;
        }
        let step = if remaining.abs() > limit {
            if remaining > 0.0 {
                limit
            } else {
                -limit
            }
        } else {
            remaining
        };
        body.x += step;
        remaining -= step;
        let (top, bottom) = rows(body, world.tile_size().1);
        if step > 0.0 {
            let column = tile_floor(body.x + body.w - 0.001, tile_w);
            if (top..=bottom).any(|row| world.solid(column, row)) {
                body.x = column as f32 * tile_w - body.w;
                body.vx = 0.0;
                hit = true;
                break;
            }
        } else {
            let column = tile_floor(body.x, tile_w);
            if (top..=bottom).any(|row| world.solid(column, row)) {
                body.x = (column + 1) as f32 * tile_w;
                body.vx = 0.0;
                hit = true;
                break;
            }
        }
    }
    hit
}

/// Move on Y, stopping at solid tiles and setting `on_ground` on landings.
pub fn move_y(body: &mut Body, dy: f32, world: &impl TileWorld) -> bool {
    let (_, tile_h) = world.tile_size();
    if !dy.is_finite() || dy == 0.0 || tile_h <= 0.0 {
        return false;
    }
    let limit = (tile_h * 0.5).max(0.01);
    let mut remaining = dy;
    let mut hit = false;
    body.on_ground = false;
    for _ in 0..256 {
        if remaining.abs() <= 0.0001 {
            break;
        }
        let step = if remaining.abs() > limit {
            if remaining > 0.0 {
                limit
            } else {
                -limit
            }
        } else {
            remaining
        };
        body.y += step;
        remaining -= step;
        let (left, right) = columns(body, world.tile_size().0);
        if step > 0.0 {
            let row = tile_floor(body.y + body.h - 0.001, tile_h);
            if (left..=right).any(|column| world.solid(column, row)) {
                body.y = row as f32 * tile_h - body.h;
                body.vy = 0.0;
                body.on_ground = true;
                hit = true;
                break;
            }
        } else {
            let row = tile_floor(body.y, tile_h);
            if (left..=right).any(|column| world.solid(column, row)) {
                body.y = (row + 1) as f32 * tile_h;
                body.vy = 0.0;
                hit = true;
                break;
            }
        }
    }
    hit
}

pub fn move_and_slide(body: &mut Body, dx: f32, dy: f32, world: &impl TileWorld) -> MoveResult {
    let hit_x = move_x(body, dx, world);
    let hit_y = move_y(body, dy, world);
    MoveResult {
        hit_x,
        hit_y,
        on_ground: body.on_ground,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 8x5 map, floor on the bottom row, a wall on the right edge.
    static TILES: [u16; 40] = [
        0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
        1, 1, 1, 1, 1, 1, 1, 1, 0, 0,
    ];

    fn world() -> SolidMap<'static> {
        SolidMap::new(TileMap::new(&TILES, 8, 5, 16, 16), 1)
    }

    #[test]
    fn horizontal_movement_stops_at_walls_and_substeps() {
        let world = world();
        let mut body = Body::new(32.0, 16.0, 12.0, 12.0);
        assert!(!move_x(&mut body, 8.0, &world));
        assert_eq!(body.x, 40.0);
        assert!(move_x(&mut body, 200.0, &world));
        assert_eq!(body.x, 7.0 * 16.0 - 12.0);
        assert_eq!(body.vx, 0.0);
        let mut body = Body::new(32.0, 16.0, 12.0, 12.0);
        assert!(!move_x(&mut body, -20.0, &world));
        assert_eq!(body.x, 12.0);
        assert!(move_x(&mut body, -40.0, &world));
        assert_eq!(body.x, 0.0);
    }

    #[test]
    fn vertical_movement_lands_sets_ground_and_jumps() {
        let world = world();
        let mut body = Body::new(40.0, 0.0, 12.0, 12.0);
        body.vy = 100.0;
        assert!(move_y(&mut body, 500.0, &world));
        assert_eq!(body.y, 4.0 * 16.0 - 12.0);
        assert!(body.on_ground);
        assert_eq!(body.vy, 0.0);
        assert!(body.jump(90.0));
        assert_eq!(body.vy, -90.0);
        assert!(!body.on_ground);
        assert!(!body.jump(90.0));
        body.apply_gravity(0.5, 200.0, 300.0);
        assert_eq!(body.vy, 10.0);
        body.apply_gravity(10.0, 200.0, 300.0);
        assert_eq!(body.vy, 300.0);
        assert!(!move_y(&mut body, -32.0, &world));
        assert_eq!(body.y, 20.0);
    }

    #[test]
    fn move_and_slide_reports_axes_and_clamps_world_edges() {
        let world = world();
        let mut body = Body::new(48.0, 32.0, 16.0, 16.0);
        let result = move_and_slide(&mut body, 4.0, 400.0, &world);
        assert!(!result.hit_x);
        assert!(result.hit_y);
        assert!(result.on_ground);
        assert_eq!(body.y, 4.0 * 16.0 - 16.0);
        body.clamp_to_world(&world);
        assert!(body.x <= 8.0 * 16.0 - 16.0);
        let mut outside = Body::new(-50.0, -50.0, 16.0, 16.0);
        outside.clamp_to_world(&world);
        assert_eq!((outside.x, outside.y), (0.0, 0.0));
        assert!(outside.vx >= 0.0 && outside.vy >= 0.0);
        let mut invalid = Body::new(0.0, 0.0, 8.0, 8.0);
        invalid.apply_gravity(f32::NAN, 10.0, 10.0);
        assert_eq!(invalid.vy, 0.0);
    }
}
