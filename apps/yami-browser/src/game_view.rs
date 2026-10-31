//! Native game presentation. The browser knows drawing commands, never game rules.
use crate::chrome::{self, Action, Canvas, ChromeState, Rect, Scene, INK};
use tiny_skia::{Pixmap, PixmapPaint, Transform};
use yami_games::{Command, Snapshot, Status, HEIGHT, WIDTH};
const MUTED: [u8; 4] = [169, 183, 207, 255];
fn color(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

#[derive(Default)]
pub struct Presentation {
    tab: u64,
    game_frame: u64,
    last: Option<std::time::Instant>,
    intervals: u64,
    elapsed: f64,
    frames: u64,
    paint_ms: f64,
}
impl Presentation {
    pub fn record(
        &mut self,
        tab: u64,
        running: bool,
        game_frame: u64,
        elapsed: std::time::Duration,
    ) {
        if tab != self.tab {
            *self = Self {
                tab,
                ..Self::default()
            };
        }
        if !running {
            self.last = None;
            return;
        }
        if self.game_frame == game_frame {
            return;
        }
        self.game_frame = game_frame;
        let now = std::time::Instant::now();
        if let Some(last) = self.last {
            self.elapsed += now.duration_since(last).as_secs_f64();
            self.intervals += 1;
        }
        self.last = Some(now);
        self.frames += 1;
        self.paint_ms += elapsed.as_secs_f64() * 1000.;
    }
    pub fn label(&self) -> String {
        format!(
            "Presented {:.1} FPS · paint + present avg {:.2} ms",
            if self.elapsed > 0. {
                self.intervals as f64 / self.elapsed
            } else {
                0.
            },
            if self.frames > 0 {
                self.paint_ms / self.frames as f64
            } else {
                0.
            }
        )
    }
}

pub fn paint(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    game: &Snapshot,
    presentation: &Presentation,
) {
    let area = chrome::viewport(ui.width, ui.height);
    c.round(area, 10., [13, 19, 36, 255], [13, 19, 36, 255], None);
    c.text(
        &game.title,
        area.x + 24.,
        area.y + 16.,
        area.w - 340.,
        20.,
        INK,
        true,
    );
    for (i, (label, action)) in [
        (
            if game.status == Status::Paused {
                "Resume"
            } else {
                "Pause"
            },
            Action::GamePause,
        ),
        ("Restart", Action::GameReset),
        ("Stop", Action::GameStop),
    ]
    .into_iter()
    .enumerate()
    {
        chrome::panel_button(
            c,
            scene,
            ui,
            Rect::new(
                area.x + area.w - 300. + i as f32 * 92.,
                area.y + 12.,
                84.,
                30.,
            ),
            label,
            action,
            true,
        );
    }
    let factor = ((area.w - 32.) / WIDTH as f32)
        .min((area.h - 150.) / HEIGHT as f32)
        .max(0.01);
    let x = area.x + (area.w - WIDTH as f32 * factor) / 2.;
    let y = area.y + 60. + ((area.h - 150.) - HEIGHT as f32 * factor) / 2.;
    if let Some(frame) = &game.frame {
        // Always draw to a separate fixed-size pixmap. Long text is clipped here,
        // so even valid guest commands cannot paint over browser chrome or controls.
        if let Some(mut pix) = Pixmap::new(WIDTH as u32, HEIGHT as u32) {
            let mut gc = Canvas::new(&mut pix, c.text, 1.);
            for command in &frame.commands {
                match command {
                    Command::Clear(value) => {
                        let v = color(*value);
                        gc.pix
                            .fill(tiny_skia::Color::from_rgba8(v[0], v[1], v[2], v[3]));
                    }
                    Command::Rect {
                        x,
                        y,
                        w,
                        h,
                        color: value,
                    } => {
                        let v = color(*value);
                        gc.round(
                            Rect::new(*x as f32, *y as f32, *w as f32, *h as f32),
                            0.,
                            v,
                            v,
                            None,
                        );
                    }
                    Command::Circle {
                        x,
                        y,
                        radius,
                        color: value,
                    } => {
                        let r = *radius as f32;
                        let v = color(*value);
                        gc.oval(
                            Rect::new(*x as f32 - r, *y as f32 - r, 2. * r, 2. * r),
                            v,
                            v,
                        );
                    }
                    Command::Text {
                        x,
                        y,
                        size,
                        text,
                        color: value,
                    } => gc.text(
                        text,
                        *x as f32,
                        *y as f32,
                        (WIDTH - x) as f32,
                        *size as f32,
                        color(*value),
                        false,
                    ),
                }
            }
            c.pix.draw_pixmap(
                0,
                0,
                pix.as_ref(),
                &PixmapPaint::default(),
                Transform::from_scale(factor * c.scale, factor * c.scale)
                    .post_translate(x * c.scale, y * c.scale),
                None,
            );
        }
    }
    let status = match &game.status {
        Status::Loading => "Loading local WebAssembly…".into(),
        Status::Paused => "PAUSED · Resume or press P".into(),
        Status::Stopped => "STOPPED · Restart to play".into(),
        Status::Failed(error) => format!("Game stopped: {error}"),
        Status::Running => {
            "LOCAL WASM · A/D or arrows to move · Space to start · P to pause".into()
        }
    };
    c.text(
        &status,
        area.x + 24.,
        area.y + area.h - 57.,
        area.w - 48.,
        13.,
        MUTED,
        false,
    );
    if game.frames > 0 {
        let info = format!(
            "ABI 1 · 640 × 480 · 60 Hz target · VM avg {:.2} ms / max {:.2} ms · {} late frames",
            game.total_update_us as f64 / game.frames as f64 / 1000.,
            game.max_update_us as f64 / 1000.,
            game.late_frames
        );
        c.text(
            &info,
            area.x + 24.,
            area.y + area.h - 32.,
            area.w - 48.,
            11.,
            MUTED,
            false,
        );
    }
    c.text(
        &presentation.label(),
        area.x + 24.,
        area.y + area.h - 79.,
        area.w - 48.,
        11.,
        MUTED,
        false,
    );
    chrome::round_viewport(c, ui);
}

pub fn launcher(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    path: &str,
    error: Option<&str>,
) {
    let area = chrome::viewport(ui.width, ui.height);
    // A modal owns every hit in the document area.
    scene.hits.retain(|hit| hit.rect.x < area.x);
    scene.add(area, Action::DiscoveryPanel);
    c.round(area, 10., [24, 30, 47, 255], [24, 30, 47, 255], None);
    let x = area.x + 32.;
    let w = area.w - 64.;
    c.text("Open a local game", x, area.y + 36., w, 26., INK, true);
    c.text(
        "Choose a .yami-game.json manifest on this computer.",
        x,
        area.y + 86.,
        w,
        15.,
        MUTED,
        false,
    );
    c.text(
        "Paste the full local path below, then press Enter to launch.",
        x,
        area.y + 117.,
        w,
        14.,
        MUTED,
        false,
    );
    let field = Rect::new(x, area.y + 165., w, 46.);
    c.round(
        field,
        6.,
        [45, 56, 79, 255],
        [45, 56, 79, 255],
        Some([92, 225, 230, 255]),
    );
    c.text(
        if path.is_empty() {
            "/path/to/game.yami-game.json"
        } else {
            path
        },
        field.x + 12.,
        field.y + 13.,
        field.w - 24.,
        14.,
        INK,
        false,
    );
    scene.add(field, Action::GamePath);
    chrome::panel_button(
        c,
        scene,
        ui,
        Rect::new(x, area.y + 237., 190., 36.),
        "Launch local game",
        Action::LaunchGame,
        true,
    );
    c.text(
        "Games run in a limited WebAssembly runtime. Only open games you choose.",
        x,
        area.y + 305.,
        w,
        13.,
        MUTED,
        false,
    );
    if let Some(error) = error {
        c.text(error, x, area.y + 349., w, 13., [255, 160, 160, 255], false);
    }
}
