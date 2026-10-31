//! Native game presentation. The browser knows drawing commands, never game rules.
use crate::chrome::{self, Action, Canvas, ChromeState, Rect, Scene, INK};
use std::sync::Arc;
use tiny_skia::{Paint, Pixmap, PixmapPaint, Transform};
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
    pub diagnostics: bool,
    surface: Option<Pixmap>,
    frame: Option<Arc<yami_games::Frame>>,
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
            self.tab = tab;
            self.game_frame = 0;
            self.last = None;
            self.intervals = 0;
            self.elapsed = 0.;
            self.frames = 0;
            self.paint_ms = 0.;
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

const BACKGROUND: [u8; 4] = [8, 12, 23, 255];
const ACCENT: [u8; 4] = [114, 235, 239, 255];
const EDGE: [u8; 4] = [42, 57, 78, 255];

/// All games share this aspect-preserving native surface and compact toolbar.
/// Coordinates returned here are logical window pixels, independent of DPI.
pub fn surface(area: Rect) -> Rect {
    let factor = ((area.w - 32.) / WIDTH as f32)
        .min((area.h - 112.) / HEIGHT as f32)
        .max(0.01);
    let w = WIDTH as f32 * factor;
    let h = HEIGHT as f32 * factor;
    Rect::new(
        area.x + (area.w - w) / 2.,
        area.y + 64. + (area.h - 112. - h) / 2.,
        w,
        h,
    )
}

#[allow(clippy::too_many_arguments)]
fn button(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    r: Rect,
    label: &str,
    action: Action,
    enabled: bool,
    primary: bool,
) {
    let hover = r.contains(ui.mouse.0, ui.mouse.1) || ui.focus == Some(&action);
    let fill = if primary {
        [25, 64, 76, 255]
    } else if enabled && hover {
        [36, 49, 69, 255]
    } else {
        [18, 26, 41, 255]
    };
    let ink = if !enabled {
        [97, 113, 137, 255]
    } else if primary {
        ACCENT
    } else {
        INK
    };
    c.round(
        r,
        8.,
        fill,
        fill,
        Some(if enabled && hover { ACCENT } else { EDGE }),
    );
    let size = 13.;
    let width = c.text.measure(label, size * c.scale, true) / c.scale;
    c.text(
        label,
        r.x + (r.w - width) / 2.,
        r.y + (r.h - size * 1.5) / 2.,
        r.w,
        size,
        ink,
        true,
    );
    if enabled {
        scene.add(r, action);
    }
}

pub fn paint(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    game: &Snapshot,
    presentation: &mut Presentation,
    immersive: bool,
) {
    let area = if immersive {
        Rect::new(0., 0., ui.width, ui.height)
    } else {
        chrome::viewport(ui.width, ui.height)
    };
    c.round(area, 0., BACKGROUND, BACKGROUND, None);
    c.text(
        "YAMI / ARCADE",
        area.x + 24.,
        area.y + 13.,
        180.,
        11.,
        ACCENT,
        true,
    );
    c.text(
        &game.title,
        area.x + 24.,
        area.y + 29.,
        area.w - 440.,
        16.,
        INK,
        true,
    );
    let available = matches!(game.status, Status::Running | Status::Paused);
    for (i, (label, action, enabled)) in [
        (
            if game.status == Status::Paused {
                "Resume"
            } else {
                "Pause"
            },
            Action::GamePause,
            available,
        ),
        ("Restart", Action::GameReset, game.status != Status::Loading),
        ("Stop", Action::GameStop, available),
        (
            if immersive { "Browser" } else { "Focus game" },
            Action::GameLayout,
            true,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        button(
            c,
            scene,
            ui,
            Rect::new(
                area.x + area.w - 400. + i as f32 * 94.,
                area.y + 16.,
                86.,
                34.,
            ),
            label,
            action,
            enabled,
            i == 0 && game.status == Status::Paused,
        );
    }
    let view = surface(area);
    c.round(
        Rect::new(view.x - 2., view.y - 2., view.w + 4., view.h + 4.),
        2.,
        EDGE,
        EDGE,
        None,
    );
    // Rasterize at the destination's physical resolution, including shaped text.
    // This isolated pixmap remains the clipping boundary for every guest command.
    // No magnified glyph bitmap or second texture filtering pass is involved.
    let width = (view.w * c.scale).floor().max(1.) as u32;
    let height = (view.h * c.scale).floor().max(1.) as u32;
    let factor = (width as f32 / WIDTH as f32).min(height as f32 / HEIGHT as f32);
    let resized = presentation
        .surface
        .as_ref()
        .is_none_or(|p| p.width() != width || p.height() != height);
    if resized {
        presentation.surface = Pixmap::new(width, height);
    }
    if let (Some(frame), Some(pix)) = (&game.frame, &mut presentation.surface) {
        if resized
            || !presentation
                .frame
                .as_ref()
                .is_some_and(|last| Arc::ptr_eq(last, frame) || last.commands == frame.commands)
        {
            let mut gc = Canvas::new(pix, c.text, factor);
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
                        // Snap straight edges to physical pixels for crisp grid
                        // lines. Curves and text retain antialiasing. This also
                        // avoids tiny-skia's fractional thin-rect AA assertion.
                        let mut paint = Paint {
                            anti_alias: false,
                            ..Paint::default()
                        };
                        paint.set_color_rgba8(v[0], v[1], v[2], v[3]);
                        if let Some(rect) = tiny_skia::Rect::from_xywh(
                            *x as f32 * factor,
                            *y as f32 * factor,
                            *w as f32 * factor,
                            *h as f32 * factor,
                        ) {
                            gc.pix.fill_rect(rect, &paint, Transform::identity(), None);
                        }
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
                        *size >= 18,
                    ),
                }
            }
            presentation.frame = Some(Arc::clone(frame));
        }
        c.pix.draw_pixmap(
            (view.x * c.scale).round() as i32,
            (view.y * c.scale).round() as i32,
            pix.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            None,
        );
    }
    let overlay = match &game.status {
        Status::Running => None,
        Status::Paused => Some((
            "INTERMISSION",
            "Game paused",
            "Your next move can wait.",
            "Resume / P",
            Action::GamePause,
        )),
        Status::Stopped => Some((
            "SESSION ENDED",
            "Game stopped",
            "Ready for another round?",
            "Restart",
            Action::GameReset,
        )),
        Status::Loading => Some((
            "YAMI ARCADE",
            "Getting ready",
            "Opening your local game…",
            "Loading…",
            Action::GameReset,
        )),
        Status::Failed(_) => Some((
            "SESSION ENDED",
            "Unable to play",
            "The game stopped safely.",
            "Try again",
            Action::GameReset,
        )),
    };
    if let Some((eyebrow, title, detail, label, action)) = overlay {
        c.round(view, 0., [5, 9, 18, 215], [5, 9, 18, 215], None);
        let panel = Rect::new(
            view.x + (view.w - 360.) / 2.,
            view.y + (view.h - 208.) / 2.,
            360.,
            208.,
        );
        c.round(panel, 12., [18, 27, 44, 255], [18, 27, 44, 255], Some(EDGE));
        c.round(
            Rect::new(panel.x + 28., panel.y, 48., 2.),
            0.,
            ACCENT,
            ACCENT,
            None,
        );
        c.text(
            eyebrow,
            panel.x + 28.,
            panel.y + 22.,
            304.,
            11.,
            ACCENT,
            true,
        );
        c.text(title, panel.x + 28., panel.y + 47., 304., 27., INK, true);
        c.text(
            detail,
            panel.x + 28.,
            panel.y + 94.,
            304.,
            14.,
            MUTED,
            false,
        );
        button(
            c,
            scene,
            ui,
            Rect::new(panel.x + 28., panel.y + 144., 150., 36.),
            label,
            action,
            game.status != Status::Loading,
            true,
        );
    }
    let footer = if presentation.diagnostics {
        format!(
            "{} · VM {:.2} ms · {} late",
            presentation.label(),
            game.total_update_us as f64 / game.frames.max(1) as f64 / 1000.,
            game.late_frames
        )
    } else if let Status::Failed(error) = &game.status {
        format!("Game stopped: {error}")
    } else {
        "A / D or ← / →  Move     SPACE  Start / serve     P / ESC  Pause     R  Restart".into()
    };
    c.text(
        &footer,
        area.x + 24.,
        area.y + area.h - 31.,
        area.w - 48.,
        12.,
        MUTED,
        false,
    );
    if !immersive {
        chrome::round_viewport(c, ui);
    }
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
