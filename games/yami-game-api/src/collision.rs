//! Allocation-free 2D collision and movement primitives for game guests.
//!
//! Values are finite `f32`s in the same 640×480 logical space as drawing.
//! Functions never call the host and use squared distances, so they also build
//! for `no_std` Wasm without a libm dependency.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_center(cx: f32, cy: f32, w: f32, h: f32) -> Self {
        Self::new(cx - w / 2.0, cy - h / 2.0, w, h)
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(self) -> f32 {
        self.y + self.h
    }

    pub fn center(self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.w.is_finite()
            && self.h.is_finite()
            && self.w > 0.0
            && self.h > 0.0
    }

    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    pub fn inflate(self, dx: f32, dy: f32) -> Self {
        Self::new(
            self.x - dx,
            self.y - dy,
            self.w + 2.0 * dx,
            self.h + 2.0 * dy,
        )
    }

    /// A half-open overlap test. Touching edges do not collide.
    pub fn intersects(self, other: Self) -> bool {
        self.is_valid()
            && other.is_valid()
            && self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }

    pub fn contains_point(self, x: f32, y: f32) -> bool {
        self.is_valid() && x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Overlap depth on each axis, or `(0, 0)` when separated.
    pub fn penetration(self, other: Self) -> (f32, f32) {
        let x = (self.right().min(other.right()) - self.x.max(other.x)).max(0.0);
        let y = (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0.0);
        if x > 0.0 && y > 0.0 {
            (x, y)
        } else {
            (0.0, 0.0)
        }
    }

    /// Smallest axis translation that separates the rectangles. Resolves the
    /// shallower axis first, which keeps platformer landings stable.
    pub fn separation(self, other: Self) -> (f32, f32) {
        let (x, y) = self.penetration(other);
        if x == 0.0 || y == 0.0 {
            return (0.0, 0.0);
        }
        if x < y {
            let direction = if self.center().0 < other.center().0 {
                -x
            } else {
                x
            };
            (direction, 0.0)
        } else {
            let direction = if self.center().1 < other.center().1 {
                -y
            } else {
                y
            };
            (0.0, direction)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Circle {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

impl Circle {
    pub const fn new(x: f32, y: f32, r: f32) -> Self {
        Self { x, y, r }
    }

    pub fn is_valid(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.r.is_finite() && self.r > 0.0
    }

    pub fn contains_point(self, x: f32, y: f32) -> bool {
        if !self.is_valid() {
            return false;
        }
        let dx = x - self.x;
        let dy = y - self.y;
        dx * dx + dy * dy <= self.r * self.r
    }

    pub fn intersects(self, other: Self) -> bool {
        if !self.is_valid() || !other.is_valid() {
            return false;
        }
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let reach = self.r + other.r;
        dx * dx + dy * dy < reach * reach
    }

    /// Circle/AABB overlap using the closest point on the rectangle.
    pub fn intersects_rect(self, rect: Rect) -> bool {
        if !self.is_valid() || !rect.is_valid() {
            return false;
        }
        let closest_x = self.x.clamp(rect.x, rect.right());
        let closest_y = self.y.clamp(rect.y, rect.bottom());
        let dx = self.x - closest_x;
        let dy = self.y - closest_y;
        dx * dx + dy * dy < self.r * self.r
    }
}

/// Move `current` toward `target` by at most `max_delta` without overshooting.
pub fn approach(current: f32, target: f32, max_delta: f32) -> f32 {
    if !current.is_finite() || !target.is_finite() || !max_delta.is_finite() {
        return current;
    }
    let delta = target - current;
    if delta.abs() <= max_delta.max(0.0) {
        target
    } else if delta > 0.0 {
        current + max_delta.max(0.0)
    } else {
        current - max_delta.max(0.0)
    }
}

/// Frame-rate-independent smoothing. `rate` is the fraction of the remaining
/// distance removed per second; values above 1 are allowed.
pub fn damp(current: f32, target: f32, rate: f32, dt: f32) -> f32 {
    if !current.is_finite() || !target.is_finite() || !rate.is_finite() || !dt.is_finite() {
        return current;
    }
    let factor = (rate * dt.max(0.0)).clamp(0.0, 1.0);
    current + (target - current) * factor
}

/// A digital direction from two held bits (for example LEFT/RIGHT).
pub fn axis(negative: bool, positive: bool) -> i8 {
    i8::from(positive) - i8::from(negative)
}

/// Clamp a signed axis to -1, 0 or 1.
pub fn sign(value: f32) -> i8 {
    if !value.is_finite() || value == 0.0 {
        0
    } else if value < 0.0 {
        -1
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangles_detect_overlap_edges_and_points() {
        let a = Rect::new(10.0, 10.0, 20.0, 10.0);
        let b = Rect::new(25.0, 15.0, 10.0, 10.0);
        assert!(a.intersects(b));
        assert!(!a.intersects(Rect::new(30.0, 10.0, 5.0, 5.0)));
        assert!(a.contains_point(10.0, 10.0));
        assert!(!a.contains_point(30.0, 20.0));
        assert_eq!(a.penetration(b), (5.0, 5.0));
        let (dx, dy) = a.separation(b);
        assert_eq!(dx, 0.0);
        assert_eq!(dy, -5.0);
        assert!(!Rect::new(0.0, 0.0, 0.0, 5.0).is_valid());
        assert!(!Rect::new(f32::NAN, 0.0, 5.0, 5.0).intersects(a));
    }

    #[test]
    fn circles_use_squared_distances_and_rect_closest_points() {
        let a = Circle::new(10.0, 10.0, 5.0);
        assert!(a.contains_point(13.0, 10.0));
        assert!(!a.contains_point(16.0, 10.0));
        assert!(a.intersects(Circle::new(16.0, 10.0, 2.0)));
        assert!(!a.intersects(Circle::new(17.0, 10.0, 1.0)));
        assert!(a.intersects_rect(Rect::new(14.0, 5.0, 10.0, 10.0)));
        assert!(!a.intersects_rect(Rect::new(20.0, 5.0, 10.0, 10.0)));
        assert!(!Circle::new(0.0, 0.0, 0.0).is_valid());
    }

    #[test]
    fn movement_helpers_do_not_overshoot_or_leak_non_finite_values() {
        assert_eq!(approach(0.0, 10.0, 3.0), 3.0);
        assert_eq!(approach(0.0, 2.0, 3.0), 2.0);
        assert_eq!(approach(10.0, 0.0, 3.0), 7.0);
        assert_eq!(approach(0.0, f32::NAN, 3.0), 0.0);
        assert_eq!(damp(0.0, 10.0, 0.5, 2.0), 10.0);
        assert_eq!(damp(0.0, 10.0, 0.1, 0.0), 0.0);
        assert_eq!(axis(true, false), -1);
        assert_eq!(axis(false, true), 1);
        assert_eq!(axis(true, true), 0);
        assert_eq!(sign(-0.0), 0);
        assert_eq!(sign(-2.0), -1);
        assert_eq!(sign(f32::INFINITY), 0);
    }
}
