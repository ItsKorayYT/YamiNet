//! Original arcade game. All simulation, screens and art execute inside Wasm.
use std::cell::RefCell;
use yami_game_api::*;
const WHITE: u32 = rgb(237, 243, 255);
const MUTED: u32 = rgb(149, 166, 200);
const CYAN: u32 = rgb(92, 225, 230);
const COLORS: [u32; 5] = [
    rgb(243, 117, 160),
    rgb(247, 158, 100),
    rgb(246, 208, 114),
    rgb(120, 211, 173),
    rgb(116, 162, 245),
];
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
        clear(rgb(13, 19, 36));
        // Subtle grid and original geometric artwork, no external assets.
        for x in (30..640).step_by(40) {
            rect(x, 60, 1, 390, rgb(21, 30, 49));
        }
        for y in (70..450).step_by(40) {
            rect(16, y, 608, 1, rgb(21, 30, 49));
        }
        text("PRISM / BREAKOUT", 24, 17, 19, WHITE);
        text(&format!("SCORE {:03}", self.score), 320, 20, 14, CYAN);
        text(&format!("LIVES {}", self.lives), 513, 20, 14, WHITE);
        rect(16, 55, 608, 2, rgb(71, 88, 119));
        rect(14, 57, 2, 393, rgb(71, 88, 119));
        rect(624, 57, 2, 393, rgb(71, 88, 119));
        for i in 0..50 {
            if !self.bricks[i] {
                continue;
            }
            let x = 30 + (i % 10) as i32 * 58;
            let y = 98 + (i / 10) as i32 * 28;
            rect(x, y, 54, 22, COLORS[i / 10]);
            rect(x + 2, y + 2, 50, 2, WHITE);
            rect(x + 2, y + 18, 50, 2, rgb(46, 48, 75));
        }
        for (i, (x, y)) in self.trail.iter().enumerate().rev() {
            circle(*x, *y, (5 - i / 2) as i32, rgb(32, 65, 88));
        }
        rect(self.paddle as i32 - 3, 408, 102, 14, rgb(28, 66, 86));
        rect(self.paddle as i32, 410, 96, 10, CYAN);
        rect(self.paddle as i32 + 8, 411, 80, 2, WHITE);
        circle(self.x as i32, self.y.clamp(66., 444.) as i32, 6, WHITE);
        text(
            "A / D  or  LEFT / RIGHT       SPACE  launch       R  reset",
            49,
            457,
            12,
            MUTED,
        );
        match self.mode {
            0 => self.panel(
                "PRISM BREAKOUT",
                "50 prisms. 3 lives. One more try.",
                "SPACE to start",
            ),
            1 => {
                text("SPACE to launch", 243, 330, 18, WHITE);
            }
            3 => self.panel(
                "GAME OVER",
                &format!("Final score: {} / 500", self.score),
                "SPACE to play again",
            ),
            4 => self.panel(
                "WALL CLEARED",
                "500 points. Every prism shattered.",
                "SPACE to play again",
            ),
            _ => {}
        }
    }
    fn panel(&self, title: &str, subtitle: &str, prompt: &str) {
        rect(94, 249, 452, 128, rgb(31, 44, 69));
        rect(94, 249, 452, 3, CYAN);
        text(title, 126, 267, 30, WHITE);
        text(subtitle, 126, 310, 16, MUTED);
        text(prompt, 126, 344, 17, CYAN);
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
