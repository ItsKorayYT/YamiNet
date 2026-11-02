//! Fixed-step sprite animation state.
//!
//! The helpers are intentionally allocation-free: clips are plain values and an
//! [`Animator`] only keeps a clip plus elapsed simulation time. They do not call
//! the host, so games may use them during update and also in tests on any target.

/// One horizontal strip of frames inside a sprite sheet.
///
/// `row` is the sheet row, `first` the first frame index in that row and
/// `frames` the number of frames. `frame_us` is the per-frame duration. A zero
/// frame count represents an empty clip; a zero duration holds the first frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clip {
    pub row: u16,
    pub first: u16,
    pub frames: u16,
    pub frame_us: u32,
    pub looping: bool,
}

impl Clip {
    pub const EMPTY: Self = Self {
        row: 0,
        first: 0,
        frames: 0,
        frame_us: 0,
        looping: true,
    };

    pub const fn new(row: u16, first: u16, frames: u16, frame_us: u32, looping: bool) -> Self {
        Self {
            row,
            first,
            frames,
            frame_us,
            looping,
        }
    }

    /// A clip that never advances. Useful for a single idle frame.
    pub const fn still(first: u16) -> Self {
        Self::new(0, first, 1, 0, true)
    }

    pub const fn is_empty(self) -> bool {
        self.frames == 0
    }

    /// Total playback time. Empty and zero-duration clips have zero duration.
    pub const fn duration_us(self) -> u64 {
        if self.frames == 0 || self.frame_us == 0 {
            0
        } else {
            self.frames as u64 * self.frame_us as u64
        }
    }

    pub const fn last_frame(self) -> u16 {
        self.first.saturating_add(self.frames.saturating_sub(1))
    }
}

/// Playback for exactly one clip at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Animator {
    clip: Clip,
    elapsed_us: u64,
}

impl Animator {
    pub const fn new(clip: Clip) -> Self {
        Self {
            clip,
            elapsed_us: 0,
        }
    }

    pub const fn clip(self) -> Clip {
        self.clip
    }

    pub const fn elapsed_us(self) -> u64 {
        self.elapsed_us
    }

    pub const fn is_empty(self) -> bool {
        self.clip.is_empty()
    }

    /// Start a clip. Passing `restart = false` keeps the current phase when the
    /// clip is unchanged, which lets a caller refresh metadata without a visual
    /// jump. A different clip always restarts at frame zero.
    pub fn set_clip(&mut self, clip: Clip, restart: bool) {
        if restart || clip != self.clip {
            self.elapsed_us = 0;
        }
        self.clip = clip;
        self.normalize();
    }

    pub fn reset(&mut self) {
        self.elapsed_us = 0;
    }

    /// Advance by microseconds. Looping clips wrap within their own duration so
    /// a long-running game cannot overflow the counter.
    pub fn advance(&mut self, dt_us: u32) {
        if self.clip.is_empty() || self.clip.frame_us == 0 {
            return;
        }
        self.elapsed_us = self.elapsed_us.saturating_add(dt_us as u64);
        self.normalize();
    }

    fn normalize(&mut self) {
        let duration = self.clip.duration_us();
        if self.clip.looping && duration != 0 {
            self.elapsed_us %= duration;
        } else if !self.clip.looping {
            self.elapsed_us = self.elapsed_us.min(duration);
        }
    }

    /// Frame index in the clip, relative to `Clip::first`, or `None` if empty.
    pub fn frame_offset(&self) -> Option<u16> {
        if self.clip.is_empty() {
            return None;
        }
        if self.clip.frame_us == 0 {
            return Some(0);
        }
        let raw = self.elapsed_us / self.clip.frame_us as u64;
        let frame = if self.clip.looping {
            raw % self.clip.frames as u64
        } else {
            raw.min(self.clip.frames as u64 - 1)
        };
        Some(frame as u16)
    }

    /// Absolute sheet frame index, or `None` for an empty clip.
    pub fn frame_index(&self) -> Option<u16> {
        self.frame_offset()
            .map(|offset| self.clip.first.saturating_add(offset))
    }

    /// True once a non-looping clip has reached its final frame. An empty clip
    /// is always finished; a zero-duration looping clip never is.
    pub fn finished(&self) -> bool {
        if self.clip.is_empty() {
            return true;
        }
        if self.clip.looping {
            return false;
        }
        if self.clip.frame_us == 0 {
            return true;
        }
        self.elapsed_us >= self.clip.duration_us()
    }

    /// `(current, total)` one-based frame numbers for HUD/debug display.
    pub fn progress(&self) -> (u16, u16) {
        match self.frame_offset() {
            Some(offset) => (offset.saturating_add(1), self.clip.frames),
            None => (0, 0),
        }
    }
}

/// A small, fixed animation-state table. Games choose the indices; the SDK only
/// handles transitions and elapsed time.
#[derive(Clone, Copy, Debug)]
pub struct AnimationState<const N: usize> {
    clips: [Clip; N],
    index: usize,
    animator: Animator,
}

impl<const N: usize> AnimationState<N> {
    pub fn new(clips: [Clip; N]) -> Self {
        let initial = clips.first().copied().unwrap_or(Clip::EMPTY);
        Self {
            clips,
            index: 0,
            animator: Animator::new(initial),
        }
    }

    pub const fn index(&self) -> usize {
        self.index
    }

    pub fn clip(&self) -> Clip {
        self.clips.get(self.index).copied().unwrap_or(Clip::EMPTY)
    }

    pub fn animator(&self) -> &Animator {
        &self.animator
    }

    pub fn clips(&self) -> &[Clip; N] {
        &self.clips
    }

    /// Select a state. Returns true when the active clip changed. Out-of-range
    /// indices are ignored so a corrupt state byte cannot stop the game.
    pub fn set(&mut self, index: usize) -> bool {
        if index >= self.clips.len() || index == self.index {
            return false;
        }
        self.index = index;
        self.animator.set_clip(self.clips[index], true);
        true
    }

    /// Select a state and report whether it changed, restarting only when the
    /// requested state differs from the current one.
    pub fn set_if_changed(&mut self, index: usize) -> bool {
        self.set(index)
    }

    pub fn advance(&mut self, dt_us: u32) {
        self.animator.advance(dt_us);
    }

    pub fn frame_index(&self) -> Option<u16> {
        self.animator.frame_index()
    }

    pub fn finished(&self) -> bool {
        self.animator.finished()
    }

    pub fn reset(&mut self) {
        self.animator.reset();
    }
}

/// Four-way facing used by directional character sheets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Facing {
    #[default]
    Down,
    Left,
    Right,
    Up,
}

impl Facing {
    pub const fn index(self) -> usize {
        match self {
            Self::Down => 0,
            Self::Left => 1,
            Self::Right => 2,
            Self::Up => 3,
        }
    }

    /// Pick a facing from a digital or analog axis. Horizontal movement wins
    /// ties so a diagonal walk does not flicker between rows.
    pub fn from_axis(x: i8, y: i8, fallback: Self) -> Self {
        if x < 0 {
            Self::Left
        } else if x > 0 {
            Self::Right
        } else if y < 0 {
            Self::Up
        } else if y > 0 {
            Self::Down
        } else {
            fallback
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looping_clips_wrap_and_do_not_overflow() {
        let mut anim = Animator::new(Clip::new(2, 8, 4, 100, true));
        assert_eq!(anim.frame_index(), Some(8));
        anim.advance(99);
        assert_eq!(anim.frame_index(), Some(8));
        anim.advance(1);
        assert_eq!(anim.frame_index(), Some(9));
        anim.advance(250);
        assert_eq!(anim.frame_index(), Some(11));
        assert!(!anim.finished());
        assert_eq!(anim.progress(), (4, 4));
        for _ in 0..1_000 {
            anim.advance(1_000_000);
        }
        assert_eq!(anim.frame_index(), Some(11));
    }

    #[test]
    fn one_shot_clips_stop_on_the_last_frame() {
        let mut anim = Animator::new(Clip::new(1, 4, 3, 200, false));
        anim.advance(10_000);
        assert_eq!(anim.frame_index(), Some(6));
        assert!(anim.finished());
        assert_eq!(anim.progress(), (3, 3));
        anim.advance(u32::MAX);
        assert_eq!(anim.frame_index(), Some(6));
    }

    #[test]
    fn empty_and_zero_duration_clips_are_stable() {
        let mut anim = Animator::new(Clip::EMPTY);
        anim.advance(1234);
        assert_eq!(anim.frame_index(), None);
        assert!(anim.finished());
        assert_eq!(anim.progress(), (0, 0));
        let mut still = Animator::new(Clip::new(0, 3, 4, 0, false));
        still.advance(1234);
        assert_eq!(still.frame_index(), Some(3));
        assert!(still.finished());
        let mut looping = Animator::new(Clip::new(0, 3, 4, 0, true));
        looping.advance(1234);
        assert_eq!(looping.frame_index(), Some(3));
        assert!(!looping.finished());
    }

    #[test]
    fn switching_clips_restarts_only_when_needed() {
        let mut anim = Animator::new(Clip::new(0, 0, 4, 100, true));
        anim.advance(150);
        assert_eq!(anim.frame_offset(), Some(1));
        anim.set_clip(Clip::new(0, 0, 4, 100, true), false);
        assert_eq!(anim.frame_offset(), Some(1));
        anim.set_clip(Clip::new(1, 4, 4, 100, true), false);
        assert_eq!(anim.frame_offset(), Some(0));
        assert_eq!(anim.frame_index(), Some(4));
    }

    #[test]
    fn animation_state_ignores_out_of_range_indices_and_tracks_changes() {
        let mut state =
            AnimationState::new([Clip::new(0, 0, 2, 100, true), Clip::new(1, 4, 2, 50, false)]);
        assert_eq!(state.index(), 0);
        assert!(!state.set(9));
        assert!(state.set(1));
        assert!(!state.set(1));
        state.advance(10_000);
        assert_eq!(state.frame_index(), Some(5));
        assert!(state.finished());
        assert_eq!(Facing::from_axis(0, -1, Facing::Down), Facing::Up);
        assert_eq!(Facing::from_axis(-1, 1, Facing::Down), Facing::Left);
        assert_eq!(Facing::from_axis(0, 0, Facing::Right), Facing::Right);
    }
}
