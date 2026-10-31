//! Original arcade game. All simulation, screens and art execute inside Wasm.
use std::cell::RefCell;
use yami_game_api::*;
const WHITE: u32 = rgb(234, 244, 255);
const MUTED: u32 = rgb(139, 160, 187);
const CYAN: u32 = rgb(114, 235, 239);
const COLORS: [u32; 5] = [
    rgb(231, 128, 199),
    rgb(184, 143, 237),
    rgb(126, 172, 241),
    rgb(103, 211, 221),
    rgb(141, 226, 193),
];
fn tint(color: u32, amount: u32) -> u32 {
    let channel = |shift: u32, base: u32| {
        ((((color >> shift) & 255) * amount + base * (100 - amount)) / 100) as u8
    };
    rgb(channel(24, 10), channel(16, 16), channel(8, 30))
}
#[derive(Clone, Copy, Default)]
struct Burst {
    x: i32,
    y: i32,
    life: f32,
    color: u32,
}
#[derive(Clone)]
struct Game {
    paddle: f32,
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    bricks: [bool; 50],
    score: u32,
    lives: u32,
    mode: u8,
    held: u32,
    pressed: u32,
    trail: [(i32, i32); 6],
    trail_tick: u32,
    bursts: [Burst; 8],
    next_burst: usize,
    paddle_flash: f32,
}
impl Default for Game {
    fn default() -> Self {
        Self {
            paddle: 272.,
            x: 320.,
            y: 393.,
            vx: 150.,
            vy: -245.,
            bricks: [true; 50],
            score: 0,
            lives: 3,
            mode: 0,
            held: 0,
            pressed: 0,
            trail: [(320, 393); 6],
            trail_tick: 0,
            bursts: [Burst::default(); 8],
            next_burst: 0,
            paddle_flash: 0.,
        }
    }
}
impl Game {
    fn serve(&mut self) {
        self.x = self.paddle + 48.;
        self.y = 393.;
        self.vx = 150.;
        self.vy = -245.;
        self.trail = [(self.x as i32, 393); 6];
    }
    fn update(&mut self, dt: f32) {
        if self.pressed & RESET != 0 {
            *self = Self::default();
        }
        if self.pressed & START != 0 {
            match self.mode {
                0 | 3 | 4 => {
                    *self = Self::default();
                    self.mode = 1
                }
                1 => self.mode = 2,
                _ => {}
            }
        }
        self.pressed = 0;
        for burst in &mut self.bursts {
            burst.life = (burst.life - dt).max(0.);
        }
        self.paddle_flash = (self.paddle_flash - dt).max(0.);
        if self.mode == 1 || self.mode == 2 {
            let direction = i32::from(self.held & RIGHT != 0) - i32::from(self.held & LEFT != 0);
            self.paddle = (self.paddle + direction as f32 * 420. * dt).clamp(20., 524.);
        }
        if self.mode == 1 {
            self.serve();
        }
        if self.mode != 2 {
            return;
        }
        // Four substeps prevent tunnelling through bricks or the paddle at 60 Hz.
        for _ in 0..4 {
            let old_y = self.y;
            self.x += self.vx * dt / 4.;
            self.y += self.vy * dt / 4.;
            if self.x < 22. {
                self.x = 22.;
                self.vx = self.vx.abs();
            }
            if self.x > 618. {
                self.x = 618.;
                self.vx = -self.vx.abs();
            }
            if self.y < 66. {
                self.y = 66.;
                self.vy = self.vy.abs();
            }
            if self.vy > 0.
                && old_y + 6. <= 410.
                && self.y + 6. >= 410.
                && self.x + 6. >= self.paddle
                && self.x - 6. <= self.paddle + 96.
            {
                self.y = 404.;
                self.paddle_flash = 0.16;
                let offset = ((self.x - self.paddle - 48.) / 48.).clamp(-1., 1.);
                self.vx = offset * 230.;
                self.vy = -(310. * 310. - self.vx * self.vx).sqrt();
            }
            for i in 0..50 {
                if !self.bricks[i] {
                    continue;
                }
                let bx = 30. + (i % 10) as f32 * 58.;
                let by = 98. + (i / 10) as f32 * 28.;
                if self.x + 6. > bx
                    && self.x - 6. < bx + 54.
                    && self.y + 6. > by
                    && self.y - 6. < by + 22.
                {
                    self.bricks[i] = false;
                    self.score += 10;
                    self.bursts[self.next_burst] = Burst {
                        x: bx as i32 + 27,
                        y: by as i32 + 11,
                        life: 0.24,
                        color: COLORS[i / 10],
                    };
                    self.next_burst = (self.next_burst + 1) % self.bursts.len();
                    if old_y + 6. <= by || old_y - 6. >= by + 22. {
                        self.vy = -self.vy;
                    } else {
                        self.vx = -self.vx;
                    }
                    break;
                }
            }
            if self.y > 450. {
                self.lives -= 1;
                self.mode = if self.lives == 0 { 3 } else { 1 };
                self.serve();
                break;
            }
            if self.bricks.iter().all(|b| !b) {
                self.mode = 4;
                break;
            }
        }
        self.trail_tick += 1;
        if self.trail_tick.is_multiple_of(2) {
            self.trail.rotate_right(1);
            self.trail[0] = (self.x as i32, self.y.clamp(66., 444.) as i32);
        }
    }
    fn draw(&self) {
        clear(rgb(10, 16, 30));
        // A quiet technical grid leaves the prismatic wall and ball in focus.
        for x in (30..624).step_by(40) {
            rect(x, 66, 1, 382, rgb(17, 26, 43));
        }
        for y in (88..448).step_by(40) {
            rect(18, y, 604, 1, rgb(17, 26, 43));
        }
        text("PRISM / 01", 28, 12, 10, CYAN);
        text("BREAKOUT", 28, 28, 18, WHITE);
        text(&format!("SCORE {:03}", self.score), 278, 18, 22, WHITE);
        text(&format!("LIVES {}", self.lives), 518, 23, 12, CYAN);
        rect(18, 58, 604, 1, rgb(44, 62, 85));
        rect(18, 58, 60, 2, CYAN);
        rect(16, 60, 1, 390, rgb(44, 62, 85));
        rect(623, 60, 1, 390, rgb(44, 62, 85));
        rect(18, 449, 604, 1, rgb(31, 47, 68));
        for i in 0..50 {
            if !self.bricks[i] {
                continue;
            }
            let x = 30 + (i % 10) as i32 * 58;
            let y = 98 + (i / 10) as i32 * 28;
            let ink = COLORS[i / 10];
            rect(x - 1, y - 1, 56, 24, tint(ink, 15));
            rect(x, y, 54, 22, tint(ink, 38));
            rect(x + 1, y + 1, 52, 2, ink);
            rect(x + 1, y + 3, 1, 17, tint(ink, 75));
            rect(x + 2, y + 19, 50, 2, tint(ink, 20));
        }
        // Fixed-size, short-lived fragments and impact flash; no random or growing buffers.
        for burst in &self.bursts {
            if burst.life <= 0. {
                continue;
            }
            let age = 1. - burst.life / 0.24;
            let spread = (4. + age * 16.) as i32;
            let ink = tint(burst.color, (burst.life / 0.24 * 100.) as u32);
            for (dx, dy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
                rect(
                    (burst.x + dx * spread).clamp(18, 618),
                    (burst.y + dy * spread / 2).clamp(62, 444),
                    3,
                    2,
                    ink,
                );
            }
        }
        if self.mode == 2 {
            for (i, (x, y)) in self.trail.iter().enumerate().rev() {
                circle(
                    *x,
                    *y,
                    (5 - i / 2) as i32,
                    tint(CYAN, 12 + (5 - i as u32) * 7),
                );
            }
        }
        let paddle = self.paddle as i32;
        rect(paddle - 3, 407, 102, 17, tint(CYAN, 12));
        rect(
            paddle,
            410,
            96,
            10,
            if self.paddle_flash > 0. { WHITE } else { CYAN },
        );
        rect(paddle + 3, 412, 90, 1, WHITE);
        rect(paddle + 8, 418, 80, 2, tint(CYAN, 55));
        let x = self.x as i32;
        let y = self.y.clamp(66., 444.) as i32;
        circle(x, y, 10, tint(CYAN, 10));
        circle(x, y, 7, tint(CYAN, 45));
        circle(x, y, 4, WHITE);
        text("50 PRISMS / 500 POINTS", 28, 459, 10, MUTED);
        text("THREE LIVES. MAKE THEM COUNT.", 397, 459, 10, MUTED);
        match self.mode {
            0 => self.panel(
                "THE ORIGINAL / YAMI ARCADE",
                "PRISM BREAKOUT",
                "50 prisms. 3 lives. One more try.",
                "SPACE to start",
            ),
            1 => {
                rect(212, 317, 216, 40, rgb(19, 37, 54));
                rect(212, 317, 2, 40, CYAN);
                text("SPACE to launch", 244, 324, 18, WHITE);
                text("A / D or arrows to position", 237, 365, 12, MUTED);
            }
            3 => self.panel(
                "END OF RUN",
                "GAME OVER",
                &format!("Final score: {} / 500", self.score),
                "SPACE to play again",
            ),
            4 => self.panel(
                "SPECTRUM COMPLETE",
                "WALL CLEARED",
                "500 points. Every prism shattered.",
                "SPACE to play again",
            ),
            _ => {}
        }
    }
    fn panel(&self, eyebrow: &str, title: &str, subtitle: &str, prompt: &str) {
        rect(98, 420, 448, 4, rgb(6, 10, 21));
        rect(542, 254, 4, 166, rgb(6, 10, 21));
        rect(94, 250, 448, 170, rgb(17, 26, 44));
        rect(94, 250, 448, 1, rgb(39, 58, 79));
        rect(94, 419, 448, 1, rgb(39, 58, 79));
        rect(94, 250, 1, 170, rgb(39, 58, 79));
        rect(541, 250, 1, 170, rgb(39, 58, 79));
        rect(126, 250, 54, 2, CYAN);
        text(eyebrow, 126, 265, 10, CYAN);
        text(title, 124, 286, 30, WHITE);
        text(subtitle, 126, 334, 14, MUTED);
        rect(126, 374, 235, 30, rgb(25, 62, 74));
        text(prompt, 142, 378, 14, CYAN);
        text("P / ESC  pause", 386, 381, 11, MUTED);
    }
}
thread_local! {static GAME:RefCell<Game>=RefCell::new(Game::default());}
#[no_mangle]
pub extern "C" fn yami_game_abi() -> i32 {
    1
}
#[no_mangle]
pub extern "C" fn yami_game_init() {
    yami_game_reset();
}
#[no_mangle]
pub extern "C" fn yami_game_reset() {
    GAME.with(|g| *g.borrow_mut() = Game::default());
}
#[no_mangle]
pub extern "C" fn yami_game_input(held: u32, pressed: u32) {
    GAME.with(|g| {
        let mut g = g.borrow_mut();
        g.held = held;
        g.pressed = pressed;
    });
}
#[no_mangle]
pub extern "C" fn yami_game_update(dt_us: i32) {
    GAME.with(|g| {
        let mut g = g.borrow_mut();
        g.update((dt_us as f32 / 1_000_000.).clamp(0., 0.02));
        g.draw();
    });
}
