//! Native game presentation. The browser knows drawing commands, never game rules.
use crate::chrome::{self, Action, Canvas, ChromeState, Rect, Scene, INK};
use std::sync::Arc;
use tiny_skia::{Paint, Pixmap, PixmapPaint, Transform};
use yami_games::{Assets, Command, Frame as GameFrame, Snapshot, Status, HEIGHT, WIDTH};
const MUTED: [u8; 4] = [169, 183, 207, 255];
fn color(value: u32) -> [u8; 4] {
    value.to_be_bytes()
}

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
    cache: Option<PrefixCache>,
    cache_enabled: bool,
}
impl Default for Presentation {
    fn default() -> Self {
        Self {
            tab: 0,
            game_frame: 0,
            last: None,
            intervals: 0,
            elapsed: 0.,
            frames: 0,
            paint_ms: 0.,
            diagnostics: false,
            surface: None,
            frame: None,
            cache: None,
            cache_enabled: true,
        }
    }
}
impl Presentation {
    #[cfg(test)]
    pub fn set_cache_enabled(&mut self, enabled: bool) {
        self.cache_enabled = enabled;
        if !enabled {
            self.cache = None;
        }
    }

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
            self.cache = None;
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

/// A prefix of an approved frame that has already been rasterized. Games usually
/// draw a static background and HUD before moving objects; reusing that prefix
/// avoids repainting every background command for each gameplay frame.
struct PrefixCache {
    prefix: usize,
    assets: Arc<Assets>,
    pixels: Pixmap,
}

/// Minimum shared-prefix length worth caching. Tiny frames just repaint.
const MIN_CACHED_PREFIX: usize = 4;
/// Upper bound for the optional prefix cache (16 MiB of RGBA pixels). Larger
/// surfaces keep the existing full-repaint path instead of doubling memory.
const MAX_CACHE_PIXELS: u64 = 4 * 1024 * 1024;

fn draw_commands(c: &mut Canvas<'_>, commands: &[Command], assets: &Assets) {
    for command in commands {
        match command {
            Command::Sprite {
                asset,
                sx,
                sy,
                sw,
                sh,
                x,
                y,
                w,
                h,
                ..
            } => {
                if let Some(texture) = assets.texture(*asset) {
                    let scale_x = *w as f32 / *sw as f32 * c.scale;
                    let scale_y = *h as f32 / *sh as f32 * c.scale;
                    let transform = Transform::from_row(
                        scale_x,
                        0.,
                        0.,
                        scale_y,
                        *x as f32 * c.scale - *sx as f32 * scale_x,
                        *y as f32 * c.scale - *sy as f32 * scale_y,
                    );
                    let paint = Paint {
                        shader: tiny_skia::Pattern::new(
                            texture.as_ref(),
                            tiny_skia::SpreadMode::Pad,
                            tiny_skia::FilterQuality::Nearest,
                            1.,
                            transform,
                        ),
                        anti_alias: false,
                        ..Paint::default()
                    };
                    if let Some(rect) = tiny_skia::Rect::from_xywh(
                        *x as f32 * c.scale,
                        *y as f32 * c.scale,
                        *w as f32 * c.scale,
                        *h as f32 * c.scale,
                    ) {
                        c.pix.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
            }
            Command::Clear(value) => {
                let v = color(*value);
                c.pix
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
                // Snap straight edges to physical pixels for crisp grid lines.
                // Curves and text retain antialiasing; this also avoids
                // tiny-skia's fractional thin-rect AA assertion.
                let mut paint = Paint {
                    anti_alias: false,
                    ..Paint::default()
                };
                paint.set_color_rgba8(v[0], v[1], v[2], v[3]);
                if let Some(rect) = tiny_skia::Rect::from_xywh(
                    *x as f32 * c.scale,
                    *y as f32 * c.scale,
                    *w as f32 * c.scale,
                    *h as f32 * c.scale,
                ) {
                    c.pix.fill_rect(rect, &paint, Transform::identity(), None);
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
                c.oval(
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
            } => c.text(
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
}

/// Rasterize a validated frame into `target`, reusing the longest common prefix
/// with the previous frame when the assets and dimensions still match.
fn render_surface(
    target: &mut Pixmap,
    text: &mut yami_render::native::TextEngine,
    frame: &GameFrame,
    previous: Option<&GameFrame>,
    cache: &mut Option<PrefixCache>,
    scale: f32,
) {
    let commands = &frame.commands;
    let common = previous
        .filter(|last| Arc::ptr_eq(&last.assets, &frame.assets))
        .map_or(0, |last| {
            last.commands
                .iter()
                .zip(commands)
                .take_while(|(old, new)| old == new)
                .count()
        });
    let usable = cache.as_ref().is_some_and(|cached| {
        cached.pixels.width() == target.width()
            && cached.pixels.height() == target.height()
            && Arc::ptr_eq(&cached.assets, &frame.assets)
            && cached.prefix <= common
    });
    if !usable {
        *cache = None;
    }
    if common >= MIN_CACHED_PREFIX && common <= commands.len() {
        let affordable = target.width() as u64 * target.height() as u64 <= MAX_CACHE_PIXELS;
        if cache.is_none() && affordable {
            if let Some(pixels) = Pixmap::new(target.width(), target.height()) {
                *cache = Some(PrefixCache {
                    prefix: 0,
                    assets: Arc::clone(&frame.assets),
                    pixels,
                });
            }
        }
        if let Some(cached) = cache.as_mut() {
            if cached.prefix < common {
                let start = cached.prefix;
                let mut canvas = Canvas::new(&mut cached.pixels, text, scale);
                draw_commands(&mut canvas, &commands[start..common], &frame.assets);
                cached.prefix = common;
            }
            target.data_mut().copy_from_slice(cached.pixels.data());
            let mut canvas = Canvas::new(target, text, scale);
            draw_commands(&mut canvas, &commands[common..], &frame.assets);
            return;
        }
    }
    *cache = None;
    let mut canvas = Canvas::new(target, text, scale);
    draw_commands(&mut canvas, commands, &frame.assets);
}

pub fn paint(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    game: &Snapshot,
    presentation: &mut Presentation,
    immersive: bool,
    controller: bool,
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
        presentation.cache = None;
    }
    if let (Some(frame), Some(pix)) = (&game.frame, &mut presentation.surface) {
        if resized
            || !presentation.frame.as_ref().is_some_and(|last| {
                Arc::ptr_eq(last, frame)
                    || (Arc::ptr_eq(&last.assets, &frame.assets) && last.commands == frame.commands)
            })
        {
            let previous = presentation.frame.clone();
            let mut disabled = None;
            let cache = if presentation.cache_enabled {
                &mut presentation.cache
            } else {
                &mut disabled
            };
            render_surface(pix, c.text, frame, previous.as_deref(), cache, factor);
            presentation.frame = Some(Arc::clone(frame));
        }
        // A clear with alpha 255 keeps the whole isolated surface opaque: every
        // validated command uses source-over, so replacing the destination is
        // pixel-identical and avoids per-pixel alpha blending for this blit.
        let opaque = matches!(
            frame.commands.first(),
            Some(Command::Clear(value)) if value & 255 == 255
        );
        let paint = PixmapPaint {
            blend_mode: if opaque {
                tiny_skia::BlendMode::Source
            } else {
                tiny_skia::BlendMode::SourceOver
            },
            ..PixmapPaint::default()
        };
        c.pix.draw_pixmap(
            (view.x * c.scale).round() as i32,
            (view.y * c.scale).round() as i32,
            pix.as_ref(),
            &paint,
            Transform::identity(),
            None,
        );
    } else {
        presentation.cache = None;
        presentation.frame = None;
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
            "Loading validated game resources…",
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
            "{} · VM {:.2} ms · {} late{}",
            presentation.label(),
            game.total_update_us as f64 / game.frames.max(1) as f64 / 1000.,
            game.late_frames,
            if controller { " · pad" } else { "" }
        )
    } else if let Status::Failed(error) = &game.status {
        format!("Game stopped: {error}")
    } else if controller {
        "Controller ready     P / ESC  Pause     R  Restart     F3  Performance".into()
    } else {
        "Controls appear in the game     P / ESC  Pause     R  Restart     F3  Performance".into()
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

#[cfg(test)]
mod tests {
    use super::*;
    use yami_render::native::TextEngine;

    fn make_frame(commands: Vec<Command>) -> GameFrame {
        GameFrame {
            commands,
            assets: Arc::new(Assets::default()),
        }
    }

    fn scene(ball_x: i32) -> GameFrame {
        make_frame(vec![
            Command::Clear(0x0a101eff),
            Command::Rect {
                x: 0,
                y: 0,
                w: 640,
                h: 480,
                color: 0x101828ff,
            },
            Command::Rect {
                x: 8,
                y: 8,
                w: 624,
                h: 46,
                color: 0x142235ff,
            },
            Command::Text {
                x: 20,
                y: 18,
                size: 20,
                text: "STATIC HEADER".into(),
                color: 0xeaf4ffff,
            },
            Command::Rect {
                x: 20,
                y: 70,
                w: 600,
                h: 2,
                color: 0x2a394aff,
            },
            Command::Circle {
                x: ball_x,
                y: 200,
                radius: 9,
                color: 0x72ebefff,
            },
            Command::Rect {
                x: ball_x - 4,
                y: 300,
                w: 8,
                h: 8,
                color: 0xffd881ff,
            },
        ])
    }

    fn render(
        frame: &GameFrame,
        previous: Option<&GameFrame>,
        cache: &mut Option<PrefixCache>,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let mut target = Pixmap::new(width, height).unwrap();
        let mut text = TextEngine::new();
        render_surface(&mut target, &mut text, frame, previous, cache, 1.0);
        target.data().to_vec()
    }

    #[test]
    fn cached_prefix_produces_pixel_identical_frames() {
        let assets = Arc::new(Assets::default());
        let mut first = scene(100);
        let mut second = scene(240);
        first.assets = Arc::clone(&assets);
        second.assets = Arc::clone(&assets);
        let mut cache = None;
        assert_eq!(
            render(&first, None, &mut cache, 640, 480),
            render(&first, None, &mut None, 640, 480)
        );
        let cached = render(&second, Some(&first), &mut cache, 640, 480);
        let fresh = render(&second, None, &mut None, 640, 480);
        assert_eq!(cached, fresh);
        assert!(cache.is_some());
        assert!(cache.as_ref().unwrap().prefix >= 5);
    }

    #[test]
    fn unchanged_frames_copy_the_cached_surface_and_resize_invalidates_it() {
        let first = scene(120);
        let mut cache = None;
        let one = render(&first, None, &mut cache, 640, 480);
        let two = render(&first, Some(&first), &mut cache, 640, 480);
        assert_eq!(one, two);
        assert_eq!(cache.as_ref().unwrap().prefix, first.commands.len());
        let resized = render(&first, Some(&first), &mut cache, 960, 720);
        assert_eq!(resized, render(&first, None, &mut None, 960, 720));
    }

    #[test]
    fn changed_assets_or_short_prefixes_fall_back_to_a_full_repaint() {
        let first = scene(100);
        let mut other = scene(100);
        other.assets = Arc::new(Assets::default());
        let mut cache = None;
        let _ = render(&first, None, &mut cache, 640, 480);
        let changed_assets = render(&other, Some(&first), &mut cache, 640, 480);
        assert_eq!(changed_assets, render(&other, None, &mut None, 640, 480));
        let short = make_frame(vec![
            Command::Clear(0x000000ff),
            Command::Rect {
                x: 0,
                y: 0,
                w: 2,
                h: 2,
                color: 0xffffffff,
            },
        ]);
        assert_eq!(
            render(&short, Some(&short), &mut None, 640, 480),
            render(&short, None, &mut None, 640, 480)
        );
    }

    #[test]
    fn text_and_sprite_layers_keep_the_validated_order() {
        let frame = make_frame(vec![
            Command::Clear(0x000000ff),
            Command::Text {
                x: 4,
                y: 4,
                size: 12,
                text: "layer zero".into(),
                color: 0xffffffff,
            },
            Command::Sprite {
                asset: 0,
                sx: 0,
                sy: 0,
                sw: 2,
                sh: 2,
                x: 10,
                y: 10,
                w: 2,
                h: 2,
                layer: 1,
            },
        ]);
        assert_eq!(
            render(&frame, None, &mut None, 640, 480),
            render(&frame, Some(&frame), &mut None, 640, 480)
        );
    }

    #[test]
    #[ignore = "manual guest-surface rendering benchmark"]
    fn measure_guest_surface_rendering() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../games/breakout/game.yami-game.json");
        let (manifest, bytes) = yami_games::load_local(&path).unwrap();
        let mut vm =
            yami_games::Runtime::with_assets(&bytes, Some(manifest.abi), manifest.prepared)
                .unwrap();
        vm.frame(0, yami_games::START).unwrap();
        vm.frame(0, yami_games::START).unwrap();
        let frames: std::vec::Vec<_> = (0..180).map(|_| vm.frame(0, 0).unwrap()).collect();
        let (width, height) = (1051, 788);
        let scale = width as f32 / WIDTH as f32;
        fn pass(
            frames: &[Arc<yami_games::Frame>],
            cached: bool,
            width: u32,
            height: u32,
            scale: f32,
        ) -> std::vec::Vec<f64> {
            let mut target = Pixmap::new(width, height).unwrap();
            let mut text = TextEngine::new();
            let mut cache = None;
            let mut disabled = None;
            let mut times = std::vec::Vec::with_capacity(frames.len());
            for (i, frame) in frames.iter().enumerate() {
                let previous = (cached && i > 0).then(|| frames[i - 1].as_ref());
                let cache_ref = if cached { &mut cache } else { &mut disabled };
                let now = std::time::Instant::now();
                render_surface(&mut target, &mut text, frame, previous, cache_ref, scale);
                times.push(now.elapsed().as_secs_f64() * 1000.);
            }
            times
        }
        // Warm text/glyph caches for both passes, then measure the steady state.
        let _ = pass(&frames, false, width, height, scale);
        let _ = pass(&frames, true, width, height, scale);
        let mut full = pass(&frames, false, width, height, scale);
        let mut cached = pass(&frames, true, width, height, scale);
        full.sort_by(f64::total_cmp);
        cached.sort_by(f64::total_cmp);
        println!(
            "guest surface {width}x{height}: full avg={:.3}ms p50={:.3}ms p95={:.3}ms; \
             cached avg={:.3}ms p50={:.3}ms p95={:.3}ms",
            full.iter().sum::<f64>() / full.len() as f64,
            full[90],
            full[171],
            cached.iter().sum::<f64>() / cached.len() as f64,
            cached[90],
            cached[171]
        );
    }
}
