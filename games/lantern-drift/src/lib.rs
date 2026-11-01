//! Lantern Drift: original art, synthesis and simulation. No native game logic.
use yami_game_api::{v2::*, *};
const INK: u32 = rgb(231, 245, 238);
const GOLD: u32 = rgb(255, 216, 129);
const DIM: u32 = rgb(132, 170, 164);
#[derive(Clone, Copy)]
struct Wisp {
    x: f32,
    y: f32,
    alive: bool,
}
struct Game {
    x: f32,
    y: f32,
    tick: u64,
    mode: u8,
    score: u32,
    hearts: u8,
    wisps: [Wisp; 8],
    cooldown: i32,
    invincible: i32,
    pulse: i32,
    muted: bool,
}
impl Default for Game {
    fn default() -> Self {
        Self {
            x: 320.,
            y: 320.,
            tick: 0,
            mode: 0,
            score: 0,
            hearts: 3,
            wisps: std::array::from_fn(|i| Wisp {
                x: 70. + (i % 4) as f32 * 160.,
                y: 140. + (i / 4) as f32 * 130.,
                alive: true,
            }),
            cooldown: 0,
            invincible: 0,
            pulse: 0,
            muted: false,
        }
    }
}
fn sprite(row: i32, size: i32, layer: i32, tick: u64, x: i32, y: i32) {
    Sprite {
        asset: 0,
        source: [0, row, 32, 32],
        size: [size, size],
        layer,
    }
    .animated(tick * 16_667, 140_000, 4)
    .draw(x, y);
}
impl Game {
    fn frame(&mut self, input: Input, _: i32) {
        if input.pressed(RESET) {
            *self = Self::default();
            Channel(0).stop();
        }
        if input.pressed(SECONDARY | MOUSE_RIGHT) {
            self.muted = !self.muted;
            mute(self.muted);
        }
        if self.mode != 1 && input.pressed(START | MOUSE_LEFT) {
            let muted = self.muted;
            *self = Self::default();
            self.mode = 1;
            self.muted = muted;
            mute(muted);
            Channel(0).play(3, 180, true);
        }
        self.tick += 1;
        if self.mode == 1 {
            self.cooldown = (self.cooldown - 1).max(0);
            self.invincible = (self.invincible - 1).max(0);
            self.pulse = (self.pulse - 1).max(0);
            let mut dx = (input.down(RIGHT) as i32 - input.down(LEFT) as i32) as f32;
            let mut dy = (input.down(DOWN) as i32 - input.down(UP) as i32) as f32;
            if input.down(MOUSE_LEFT) {
                if let Some((x, y)) = input.pointer() {
                    let vx = x as f32 - self.x;
                    let vy = y as f32 - self.y;
                    let length = (vx * vx + vy * vy).sqrt();
                    if length > 4. {
                        dx = vx / length;
                        dy = vy / length;
                    }
                }
            }
            let length = (dx * dx + dy * dy).sqrt().max(1.);
            self.x = (self.x + dx / length * 3.3).clamp(24., 616.);
            self.y = (self.y + dy / length * 3.3).clamp(94., 418.);
            if input.pressed(START | ACTION) && self.cooldown == 0 {
                self.pulse = 22;
                self.cooldown = 90;
                Channel(1).play(1, 400, false);
            }
            for (i, wisp) in self.wisps.iter_mut().enumerate() {
                if !wisp.alive {
                    wisp.x = 48. + ((self.score as usize * 97 + i * 137) % 544) as f32;
                    wisp.y = 110. + ((self.score as usize * 71 + i * 89) % 290) as f32;
                    wisp.alive = true;
                }
                let distance = ((self.x - wisp.x).powi(2) + (self.y - wisp.y).powi(2)).sqrt();
                if distance < if self.pulse > 0 { 92. } else { 28. } {
                    wisp.alive = false;
                    self.score += 1;
                    Channel(2).play(1, 550, false);
                }
            }
            for i in 0..4 {
                let (x, y) = self.moth(i);
                if (self.x - x).powi(2) + (self.y - y).powi(2) < 28_f32.powi(2)
                    && self.invincible == 0
                {
                    self.hearts -= 1;
                    self.invincible = 120;
                    Channel(3).play(2, 500, false);
                }
            }
            if self.score >= 24 {
                self.mode = 2;
                Channel(0).stop();
                Channel(4).play(1, 700, false);
            } else if self.hearts == 0 || self.tick >= 60 * 50 {
                self.mode = 3;
                Channel(0).stop();
                Channel(4).play(2, 500, false);
            }
        }
        self.draw();
    }
    fn moth(&self, i: u32) -> (f32, f32) {
        let t = self.tick as f32 * 0.018 + i as f32 * 1.7;
        (320. + t.sin() * 255., 240. + (t * 1.3).cos() * 125.)
    }
    fn draw(&self) {
        clear(rgb(9, 23, 31));
        // Water bands and distant reeds; primitive commands remain layer 0.
        for i in 0..11 {
            rect(0, 85 + i * 30, 640, 1, rgb(17, 41, 48));
        }
        for i in 0..18 {
            let x = 12 + i * 36;
            rect(x, 390 + (i % 3) * 6, 2, 42, rgb(30, 69, 64));
        }
        if self.mode == 1 {
            for (i, wisp) in self.wisps.iter().enumerate() {
                if wisp.alive {
                    sprite(
                        32,
                        40,
                        1,
                        self.tick + i as u64 * 9,
                        wisp.x as i32 - 20,
                        wisp.y as i32 - 20,
                    );
                }
            }
            for i in 0..4 {
                let (x, y) = self.moth(i);
                sprite(
                    64,
                    48,
                    2,
                    self.tick + i as u64 * 7,
                    x as i32 - 24,
                    y as i32 - 24,
                );
            }
            if self.pulse > 0 {
                let radius = 30 + (22 - self.pulse) * 3;
                circle(
                    self.x as i32,
                    self.y as i32,
                    radius
                        .min(self.x as i32 - 1)
                        .min(639 - self.x as i32)
                        .min(self.y as i32 - 1)
                        .min(479 - self.y as i32)
                        .max(1),
                    0x68eacb35,
                );
            }
            if self.invincible % 12 < 7 {
                sprite(0, 64, 3, self.tick, self.x as i32 - 32, self.y as i32 - 32);
            }
        } else {
            sprite(0, 64, 3, self.tick, 288, 78);
        }
        rect(0, 0, 640, 74, rgb(15, 35, 43));
        text("LANTERN / DRIFT", 24, 15, 23, GOLD);
        text(
            &format!(
                "LIGHT {:02}/24    HEARTS {}    TIME {:02}",
                self.score,
                self.hearts,
                50_u64.saturating_sub(self.tick / 60)
            ),
            24,
            47,
            13,
            INK,
        );
        text(
            if self.muted { "SILENT" } else { "SOUND ON" },
            536,
            24,
            11,
            DIM,
        );
        rect(0, 440, 640, 40, rgb(15, 35, 43));
        text(
            "WASD / arrows or hold mouse: steer   SPACE / Z: pulse",
            18,
            445,
            12,
            DIM,
        );
        text(
            "X / right click: mute   R: restart   P / ESC: pause",
            18,
            462,
            10,
            DIM,
        );
        if self.mode != 1 {
            rect(70, 145, 500, 230, rgb(19, 42, 50));
            rect(70, 145, 500, 2, GOLD);
            let (title, detail) = match self.mode {
                2 => (
                    "THE MARSH IS ALIGHT",
                    "24 lights gathered. A safe passage home.",
                ),
                3 => (
                    "THE LIGHT FADES",
                    "Watch the moths. Pulse gathers nearby lights.",
                ),
                _ => (
                    "LANTERN DRIFT",
                    "Gather 24 wandering lights before night falls.",
                ),
            };
            text("A SMALL JOURNEY THROUGH THE NIGHT", 98, 170, 11, GOLD);
            text(title, 96, 199, 30, INK);
            text(detail, 98, 252, 14, DIM);
            text(
                "Steer around the moths. You have three hearts.",
                98,
                279,
                13,
                DIM,
            );
            text("CLICK / SPACE TO SET SAIL", 113, 328, 19, GOLD);
        }
    }
}
yami_game_api::export_game!(Game);
