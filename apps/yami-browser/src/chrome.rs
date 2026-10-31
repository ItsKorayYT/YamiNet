#![allow(clippy::too_many_arguments)]
//! The browser theme is independent of YSL document presentation.
use crate::model::{Browser, PageState, Security};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, SpreadMode, Stroke, Transform,
};
use yami_render::native::TextEngine;
mod discover;
#[cfg(test)]
pub use discover::discover_rect;
pub use discover::{discover_page_size, discover_panel};

// Shared native palette: charcoal surfaces, plum accents and warm, readable ink.
pub const INK: [u8; 4] = [242, 236, 238, 255];
const MUTED: [u8; 4] = [184, 170, 180, 255];
const SIDEBAR_BG: [u8; 4] = [25, 22, 29, 255];
const MAIN_BG: [u8; 4] = [32, 28, 37, 255];
const PLUM_GLOW: [u8; 4] = [117, 80, 110, 255];
const HOVER_BG: [u8; 4] = [41, 35, 47, 255];
const ACTIVE_TAB: [u8; 4] = [64, 47, 65, 255];
const EDGE: [u8; 4] = [66, 55, 73, 255];
const ACCENT: [u8; 4] = [218, 177, 207, 255];
const VERIFIED: [u8; 4] = [156, 204, 170, 255];
const PLAINTEXT: [u8; 4] = [238, 174, 151, 255];

pub const SIDEBAR_WIDTH: f32 = 248.0;
pub const MARGIN: f32 = 10.0;
const RADIUS: f32 = 14.0;
mod pages;
pub use pages::{error_details_rect, error_page, error_scroll_limit, home};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    OpenGame,
    GamePath,
    LaunchGame,
    GamePause,
    GameReset,
    GameStop,
    GameLayout,
    Discover,
    DiscoverView(bool),
    DiscoverMode(bool),
    DiscoverCancel,
    DiscoverField(yami_browser::discover::Field),
    DiscoverSearch,
    DiscoverAdd,
    DiscoverSource(String),
    DiscoverWebsite(String),
    DiscoverRefresh,
    DiscoverRemove,
    DiscoverOpen,
    DiscoverPage(bool),
    Settings,
    FinishSetup,
    ToggleLocal,
    ResetSettings,
    Find,
    FindNext(bool),
    CloseFind,
    Follow(String),
    Library,
    SaveBookmark,
    OpenBookmark(usize),
    RemoveBookmark(usize),
    EditBookmark(usize, bool),
    SetHomepage,
    ToggleHistory,
    ClearHistory,
    OpenVisit(usize),
    LibraryPage(bool),
    FindLocal,
    ToggleRelay,
    Bootstrap,
    SelectLan,
    SearchLan,
    LocalNode(usize),
    OpenLocal,
    LocalPage(bool),
    DiscoveryPanel,
    Tab(usize),
    Close(usize),
    NewTab,
    Back,
    Forward,
    Reload,
    Address,
    Home,
    Navigate(&'static str),
    Scrollbar,
}
#[derive(Clone)]
pub struct Hit {
    pub rect: Rect,
    pub action: Action,
}
#[derive(Default)]
pub struct Scene {
    pub hits: Vec<Hit>,
}
impl Scene {
    pub fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.rect.contains(x, y))
            .map(|h| h.action.clone())
    }
    pub(crate) fn add(&mut self, rect: Rect, action: Action) {
        self.hits.push(Hit { rect, action });
    }
}

pub struct Canvas<'a> {
    pub pix: &'a mut Pixmap,
    pub text: &'a mut TextEngine,
    pub scale: f32,
}
impl<'a> Canvas<'a> {
    pub fn new(pix: &'a mut Pixmap, text: &'a mut TextEngine, scale: f32) -> Self {
        Self { pix, text, scale }
    }

    pub fn round(
        &mut self,
        r: Rect,
        radius: f32,
        top: [u8; 4],
        bottom: [u8; 4],
        border: Option<[u8; 4]>,
    ) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        if let Some(path) = rounded_path(r, radius) {
            // Solid surfaces dominate continuous game painting. Sending equal
            // stops through a gradient shader needlessly evaluates every pixel.
            let mut paint = Paint::default();
            if top == bottom {
                paint.set_color(color(top));
            } else {
                paint.shader = LinearGradient::new(
                    Point::from_xy(r.x, r.y),
                    Point::from_xy(r.x, r.y + r.h),
                    vec![
                        GradientStop::new(0.0, color(top)),
                        GradientStop::new(1.0, color(bottom)),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap();
            }
            self.pix.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::from_scale(self.scale, self.scale),
                None,
            );
            if let Some(edge) = border {
                paint.set_color(color(edge));
                self.pix.stroke_path(
                    &path,
                    &paint,
                    &Stroke {
                        width: 1.0,
                        ..Stroke::default()
                    },
                    Transform::from_scale(self.scale, self.scale),
                    None,
                );
            }
        }
    }

    /// Fills the rounded rect `r` with a shader; used for the new-tab surface.
    fn fill_shader(&mut self, r: Rect, radius: f32, shader: tiny_skia::Shader<'_>) {
        if let Some(path) = rounded_path(r, radius) {
            let paint = Paint {
                shader,
                ..Paint::default()
            };
            self.pix.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::from_scale(self.scale, self.scale),
                None,
            );
        }
    }
    /// A soft radial glow centred at (`cx`, `cy`), clipped to the rounded rect `r`.
    pub fn glow(&mut self, r: Rect, radius: f32, cx: f32, cy: f32, reach: f32, ink: [u8; 4]) {
        let edge = [ink[0], ink[1], ink[2], 0];
        let mid = [ink[0], ink[1], ink[2], ink[3] / 2];
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(cx, cy),
            Point::from_xy(cx, cy),
            reach.max(1.0),
            vec![
                GradientStop::new(0.0, color(ink)),
                GradientStop::new(0.45, color(mid)),
                GradientStop::new(1.0, color(edge)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            self.fill_shader(r, radius, shader);
        }
    }
    /// Repaints the square corners outside a rounded rect with `bg`, so content
    /// drawn edge-to-edge (YLang documents) reads as a rounded surface.
    pub fn mask_corners(&mut self, r: Rect, radius: f32, bg: [u8; 4]) {
        let radius = radius.min(r.w / 2.0).min(r.h / 2.0);
        let corners = [
            ((r.x, r.y), (r.x + radius, r.y), (r.x, r.y + radius)),
            (
                (r.x + r.w, r.y),
                (r.x + r.w, r.y + radius),
                (r.x + r.w - radius, r.y),
            ),
            (
                (r.x + r.w, r.y + r.h),
                (r.x + r.w - radius, r.y + r.h),
                (r.x + r.w, r.y + r.h - radius),
            ),
            (
                (r.x, r.y + r.h),
                (r.x, r.y + r.h - radius),
                (r.x + radius, r.y + r.h),
            ),
        ];
        let mut paint = Paint::default();
        paint.set_color(color(bg));
        for (corner, a, b) in corners {
            let mut path = PathBuilder::new();
            path.move_to(corner.0, corner.1);
            path.line_to(a.0, a.1);
            path.quad_to(corner.0, corner.1, b.0, b.1);
            path.close();
            if let Some(path) = path.finish() {
                self.pix.fill_path(
                    &path,
                    &paint,
                    FillRule::Winding,
                    Transform::from_scale(self.scale, self.scale),
                    None,
                );
            }
        }
    }
    pub fn oval(&mut self, r: Rect, top: [u8; 4], bottom: [u8; 4]) {
        if let Some(rect) = tiny_skia::Rect::from_xywh(r.x, r.y, r.w, r.h) {
            let Some(path) = PathBuilder::from_oval(rect) else {
                return;
            };
            let paint = Paint {
                shader: LinearGradient::new(
                    Point::from_xy(r.x, r.y),
                    Point::from_xy(r.x, r.y + r.h),
                    vec![
                        GradientStop::new(0.0, color(top)),
                        GradientStop::new(1.0, color(bottom)),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                )
                .unwrap(),
                ..Paint::default()
            };
            self.pix.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::from_scale(self.scale, self.scale),
                None,
            );
        }
    }
    pub fn line(&mut self, points: &[(f32, f32)], ink: [u8; 4], width: f32) {
        if points.is_empty() {
            return;
        }
        let mut b = PathBuilder::new();
        b.move_to(points[0].0, points[0].1);
        for p in &points[1..] {
            b.line_to(p.0, p.1);
        }
        if let Some(path) = b.finish() {
            let mut paint = Paint::default();
            paint.set_color(color(ink));
            self.pix.stroke_path(
                &path,
                &paint,
                &Stroke {
                    width,
                    line_cap: tiny_skia::LineCap::Round,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Stroke::default()
                },
                Transform::from_scale(self.scale, self.scale),
                None,
            );
        }
    }
    pub fn text(
        &mut self,
        label: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        ink: [u8; 4],
        bold: bool,
    ) {
        let ink = ysl::Color(ink[0], ink[1], ink[2]);
        if bold {
            self.text.draw_bold(
                self.pix,
                label,
                x * self.scale,
                y * self.scale,
                width * self.scale,
                size * self.scale,
                ink,
            );
        } else {
            self.text.draw(
                self.pix,
                label,
                x * self.scale,
                y * self.scale,
                width * self.scale,
                size * self.scale,
                ink,
            );
        }
    }
    pub fn icon(&mut self, kind: &str, x: f32, y: f32, ink: [u8; 4]) {
        match kind {
            "back" => {
                self.line(&[(x + 5.0, y - 5.0), (x, y), (x + 5.0, y + 5.0)], ink, 1.8);
            }
            "forward" => {
                self.line(&[(x, y - 5.0), (x + 5.0, y), (x, y + 5.0)], ink, 1.8);
            }
            "reload" => {
                self.line(
                    &[
                        (x + 9.0, y - 2.0),
                        (x + 6.0, y - 5.0),
                        (x + 2.0, y - 4.0),
                        (x, y - 1.0),
                        (x + 1.0, y + 3.0),
                        (x + 5.0, y + 5.0),
                        (x + 9.0, y + 2.0),
                    ],
                    ink,
                    1.6,
                );
                self.line(
                    &[(x + 9.0, y - 6.0), (x + 9.0, y - 1.0), (x + 5.0, y - 1.0)],
                    ink,
                    1.6,
                );
            }
            "close" => {
                self.line(&[(x, y - 3.0), (x + 6.0, y + 3.0)], ink, 1.4);
                self.line(&[(x + 6.0, y - 3.0), (x, y + 3.0)], ink, 1.4);
            }
            "plus" => {
                self.line(&[(x, y), (x + 8.0, y)], ink, 1.6);
                self.line(&[(x + 4.0, y - 4.0), (x + 4.0, y + 4.0)], ink, 1.6);
            }
            "orb" => {
                self.oval(
                    Rect::new(x - 5.0, y - 5.0, 10.0, 10.0),
                    [PLUM_GLOW[0], PLUM_GLOW[1], PLUM_GLOW[2], 200],
                    [PLUM_GLOW[0], PLUM_GLOW[1], PLUM_GLOW[2], 255],
                );
            }
            // Yami mark: a crescent made of two overlapping discs.
            "yami" => {
                self.oval(
                    Rect::new(x - 6.0, y - 6.0, 12.0, 12.0),
                    [196, 150, 166, 255],
                    PLUM_GLOW,
                );
                self.oval(
                    Rect::new(x - 3.0, y - 7.5, 11.0, 11.0),
                    SIDEBAR_BG,
                    SIDEBAR_BG,
                );
            }
            "search" => {
                let ring: Vec<(f32, f32)> = (0..=24)
                    .map(|i| {
                        let a = i as f32 / 24.0 * std::f32::consts::TAU;
                        (x + 3.5 * a.cos(), y - 1.0 + 3.5 * a.sin())
                    })
                    .collect();
                self.line(&ring, ink, 1.3);
                self.line(&[(x + 2.6, y + 1.6), (x + 5.0, y + 4.0)], ink, 1.3);
            }
            "home" => {
                self.line(
                    &[
                        (x - 5.0, y),
                        (x, y - 5.0),
                        (x + 5.0, y),
                        (x + 4.0, y),
                        (x + 4.0, y + 5.0),
                        (x - 4.0, y + 5.0),
                        (x - 4.0, y),
                    ],
                    ink,
                    1.3,
                );
            }
            "info" => {
                let ring: Vec<(f32, f32)> = (0..=32)
                    .map(|i| {
                        let a = i as f32 / 32.0 * std::f32::consts::TAU;
                        (x + 6.0 * a.cos(), y + 6.0 * a.sin())
                    })
                    .collect();
                self.line(&ring, ink, 1.3);
                self.line(&[(x, y - 0.5), (x, y + 3.0)], ink, 1.4);
                self.line(&[(x, y - 3.0), (x, y - 2.9)], ink, 1.6);
            }
            // Closed padlock for a verified node.
            "lock" => {
                self.round(Rect::new(x - 4.0, y - 1.0, 8.0, 6.5), 1.5, ink, ink, None);
                let shackle: Vec<(f32, f32)> = (0..=12)
                    .map(|i| {
                        let a = std::f32::consts::PI * (1.0 + i as f32 / 12.0);
                        (x + 2.6 * a.cos(), y - 1.0 + 3.4 * a.sin())
                    })
                    .collect();
                self.line(&shackle, ink, 1.3);
            }
            // Three linked nodes for a YNP peer.
            "node" => {
                let pts = [(x - 5.0, y + 4.0), (x, y - 5.0), (x + 5.0, y + 4.0)];
                self.line(&[pts[0], pts[1], pts[2], pts[0]], ink, 1.1);
                for (px, py) in pts {
                    self.oval(Rect::new(px - 2.0, py - 2.0, 4.0, 4.0), ink, ink);
                }
            }
            _ => {}
        }
    }
}
fn color(rgba: [u8; 4]) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}
fn rounded_path(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    if r.w <= 0.0 || r.h <= 0.0 {
        return None;
    }
    let radius = radius.min(r.w / 2.0).min(r.h / 2.0);
    let mut path = PathBuilder::new();
    path.move_to(r.x + radius, r.y);
    path.line_to(r.x + r.w - radius, r.y);
    path.quad_to(r.x + r.w, r.y, r.x + r.w, r.y + radius);
    path.line_to(r.x + r.w, r.y + r.h - radius);
    path.quad_to(r.x + r.w, r.y + r.h, r.x + r.w - radius, r.y + r.h);
    path.line_to(r.x + radius, r.y + r.h);
    path.quad_to(r.x, r.y + r.h, r.x, r.y + r.h - radius);
    path.line_to(r.x, r.y + radius);
    path.quad_to(r.x, r.y, r.x + radius, r.y);
    path.close();
    path.finish()
}

pub struct ChromeState<'a> {
    pub width: f32,
    pub height: f32,
    pub mouse: (f32, f32),
    pub editing: bool,
    pub value: &'a str,
    pub selection: bool,
    pub selection_range: Option<std::ops::Range<usize>>,
    pub caret: usize,
    pub focus: Option<&'a Action>,
}

/// Built-in shortcuts and the explicit local demo, never fetched automatically.
const PINS: [(&str, &str); 3] = [
    ("yami", "local:welcome"),
    ("node", "ynp://127.0.0.1:7301/welcome"),
    ("info", "local:about"),
];
const PAD: f32 = 12.0;
const ROW: f32 = 34.0;

/// The content surface: everything right of the sidebar, inset by the window margin.
pub fn viewport(width: f32, height: f32) -> Rect {
    Rect::new(
        SIDEBAR_WIDTH,
        MARGIN,
        (width - SIDEBAR_WIDTH - MARGIN).max(1.0),
        (height - 2.0 * MARGIN).max(1.0),
    )
}

/// Vertically centres a single line of `size` text (line box is 1.5 × size) in a row.
fn text_y(row: Rect, size: f32) -> f32 {
    row.y + (row.h - size * 1.5) / 2.0
}

fn ghost(c: &mut Canvas<'_>, r: Rect, active: bool, hovered: bool) {
    let fill = if active {
        ACTIVE_TAB
    } else if hovered {
        HOVER_BG
    } else {
        return;
    };
    c.round(r, 7.0, fill, fill, None);
}

pub fn paint(c: &mut Canvas<'_>, browser: &Browser, ui: &ChromeState<'_>) -> Scene {
    let mut scene = Scene::default();
    let (w, h) = (ui.width, ui.height);
    let hover = |r: Rect| r.contains(ui.mouse.0, ui.mouse.1);

    // The window itself is the sidebar colour; the content surface is inset on top.
    c.round(Rect::new(0.0, 0.0, w, h), 0.0, SIDEBAR_BG, SIDEBAR_BG, None);
    let left = PAD;
    let inner = SIDEBAR_WIDTH - 2.0 * PAD;

    // Top row: Yami mark on the left, compact navigation on the right.
    let top = Rect::new(left, MARGIN + 6.0, inner, 32.0);
    c.icon("yami", top.x + 9.0, top.y + top.h / 2.0, INK);
    c.text(
        "Yami",
        top.x + 25.0,
        text_y(top, 15.0),
        65.0,
        15.0,
        INK,
        true,
    );
    for (i, (action, icon, enabled)) in [
        (Action::Back, "back", browser.active().can_back()),
        (Action::Forward, "forward", browser.active().can_forward()),
        (Action::Reload, "reload", true),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(top.x + top.w - 102.0 + i as f32 * 34.0, top.y, 32.0, 32.0);
        ghost(
            c,
            r,
            false,
            enabled && (hover(r) || ui.focus == Some(&action)),
        );
        let ink = if enabled { MUTED } else { [86, 76, 91, 255] };
        let ix = if icon == "back" {
            r.x + 10.0
        } else {
            r.x + 8.0
        };
        c.icon(icon, ix, r.y + r.h / 2.0, ink);
        if enabled {
            scene.add(r, action);
        }
    }

    // Address / search field.
    let address = Rect::new(left, top.y + top.h + 12.0, inner, ROW);
    address_field(c, &mut scene, address, Action::Address, ui, ui.editing);
    let below = connection_status(c, browser.active().security(), left, address.y + address.h);

    // Pinned sites.
    let gap = 6.0;
    let pin_w = (inner - 2.0 * gap) / 3.0;
    let pins_y = below + 10.0;
    for (i, (icon, target)) in PINS.into_iter().enumerate() {
        let r = Rect::new(left + i as f32 * (pin_w + gap), pins_y, pin_w, 36.0);
        let action = Action::Navigate(target);
        let lit = hover(r) || ui.focus == Some(&action);
        let fill = if lit { ACTIVE_TAB } else { HOVER_BG };
        c.round(r, 8.0, fill, fill, None);
        c.icon(icon, r.x + r.w / 2.0, r.y + r.h / 2.0, MUTED);
        scene.add(r, action);
    }

    // Open tabs.
    let label = Rect::new(left, pins_y + 36.0 + 14.0, inner, 26.0);
    c.text(
        "OPEN TABS",
        label.x + 8.0,
        text_y(label, 11.0),
        inner - 16.0,
        11.0,
        MUTED,
        true,
    );
    // Share the existing section heading so Discover does not consume tab space.
    panel_button(
        c,
        &mut scene,
        ui,
        Rect::new(label.x + 102.0, label.y, label.w - 102.0, label.h),
        "Discover",
        Action::Discover,
        true,
    );
    let mut y = label.y + label.h + 2.0;
    // Page a crowded tab list around the active tab. Keep full-sized rows and
    // reserve space for the new-tab button, games and fixed utility group.
    let visible = (((h - 202.0 - y - 96.0) / (ROW + 4.0)).floor() as usize).max(1);
    let first = browser.active / visible * visible;
    let end = (first + visible).min(browser.tabs.len());
    for (index, tab) in browser.tabs.iter().enumerate().take(end).skip(first) {
        let r = Rect::new(left, y, inner, ROW);
        let active = index == browser.active;
        let focused = ui.focus == Some(&Action::Tab(index));
        ghost(c, r, active, hover(r) || focused);
        if active {
            c.round(
                Rect::new(r.x, r.y + 5.0, 3.0, r.h - 10.0),
                1.5,
                ACCENT,
                ACCENT,
                None,
            );
        }
        if focused {
            c.round(r, 7.0, ACTIVE_TAB, ACTIVE_TAB, Some(ACCENT));
        }
        let dot = if matches!(tab.state, PageState::Loading) {
            [196, 150, 166, 255]
        } else if active {
            MUTED
        } else {
            [120, 108, 113, 255]
        };
        c.oval(
            Rect::new(r.x + 10.0, r.y + r.h / 2.0 - 3.0, 6.0, 6.0),
            dot,
            dot,
        );
        c.text(
            tab.title(),
            r.x + 26.0,
            text_y(r, 13.0_f32.min(r.h / 1.5)),
            r.w - 58.0,
            13.0_f32.min(r.h / 1.5),
            if active { INK } else { MUTED },
            false,
        );
        scene.add(r, Action::Tab(index));
        // The close control only appears where it is useful, but stays hit-testable
        // so keyboard focus can still reach it.
        let close = Rect::new(r.x + r.w - 28.0, r.y + (r.h - 16.0) / 2.0, 24.0, 16.0);
        let close_focused = ui.focus == Some(&Action::Close(index));
        if hover(close) || close_focused {
            c.round(close, 4.0, HOVER_BG, HOVER_BG, None);
        }
        if active || hover(r) || close_focused {
            c.icon("close", close.x + 5.0, close.y + 8.0, MUTED);
        }
        scene.add(close, Action::Close(index));
        y += ROW + 4.0;
    }

    if browser.tabs.len() > visible {
        let bw = (inner - 8.0) / 2.0;
        panel_button(
            c,
            &mut scene,
            ui,
            Rect::new(left, y, bw, 24.0),
            "Earlier tabs",
            Action::Tab(first.saturating_sub(1)),
            first > 0,
        );
        panel_button(
            c,
            &mut scene,
            ui,
            Rect::new(left + bw + 8.0, y, bw, 24.0),
            "Later tabs",
            Action::Tab(end),
            end < browser.tabs.len(),
        );
        y += 28.0;
    }

    // New tab.
    let new_tab = Rect::new(left, y, inner, ROW);
    if browser.tabs.len() < 12 {
        ghost(
            c,
            new_tab,
            false,
            hover(new_tab) || ui.focus == Some(&Action::NewTab),
        );
        c.icon("plus", new_tab.x + 9.0, new_tab.y + new_tab.h / 2.0, MUTED);
        c.text(
            "New Tab",
            new_tab.x + 26.0,
            text_y(new_tab, 12.5),
            inner - 34.0,
            12.5,
            MUTED,
            false,
        );
        scene.add(new_tab, Action::NewTab);
    }

    panel_button(
        c,
        &mut scene,
        ui,
        Rect::new(left, y + ROW + 4.0, inner, 30.0),
        "Open local game",
        Action::OpenGame,
        true,
    );

    c.line(&[(left, h - 188.0), (left + inner, h - 188.0)], EDGE, 1.0);
    // Fixed utility group, separate from browsing and tabs.
    let home = Rect::new(left, h - MARGIN - 32.0, 26.0, 26.0);
    ghost(
        c,
        home,
        false,
        hover(home) || ui.focus == Some(&Action::Home),
    );
    c.icon("home", home.x + 13.0, home.y + 13.0, MUTED);
    scene.add(home, Action::Home);

    let find = Rect::new(left + 32.0, home.y, inner - 32.0, 26.0);
    ghost(
        c,
        find,
        false,
        hover(find) || ui.focus == Some(&Action::FindLocal),
    );
    c.text(
        "Find local nodes",
        find.x + 5.0,
        text_y(find, 12.0),
        find.w - 10.0,
        12.0,
        MUTED,
        false,
    );
    scene.add(find, Action::FindLocal);

    scene
}

/// Explicit relay opt-in beside the existing local-discovery utility.
pub fn relay_setting(c: &mut Canvas<'_>, scene: &mut Scene, ui: &ChromeState<'_>, enabled: bool) {
    let r = Rect::new(
        PAD,
        ui.height - MARGIN - 64.0,
        SIDEBAR_WIDTH - 2.0 * PAD,
        26.0,
    );
    ghost(c, r, false, r.contains(ui.mouse.0, ui.mouse.1));
    c.text(
        if enabled {
            "Relay fallback: On"
        } else {
            "Relay fallback: Off"
        },
        r.x + 5.0,
        text_y(r, 12.0),
        r.w - 10.0,
        12.0,
        MUTED,
        false,
    );
    scene.add(r, Action::ToggleRelay);
    let r = Rect::new(r.x, r.y - 28.0, r.w, r.h);
    ghost(c, r, false, r.contains(ui.mouse.0, ui.mouse.1));
    c.text(
        "Add bootstrap peer",
        r.x + 5.0,
        text_y(r, 12.0),
        r.w - 10.0,
        12.0,
        MUTED,
        false,
    );
    scene.add(r, Action::Bootstrap);
    let r = Rect::new(r.x, r.y - 28.0, r.w, r.h);
    panel_button(
        c,
        scene,
        ui,
        r,
        "Bookmarks & history",
        Action::Library,
        true,
    );
    let r = Rect::new(r.x, r.y - 28.0, r.w, r.h);
    panel_button(c, scene, ui, r, "Settings", Action::Settings, true);
}

pub fn settings_panel(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    settings: yami_browser::library::Settings,
    error: Option<&str>,
) {
    let r = Rect::new(
        SIDEBAR_WIDTH + 24.0,
        32.0,
        (ui.width - SIDEBAR_WIDTH - 48.0).min(750.0),
        275.0,
    );
    c.round(r, 10.0, SIDEBAR_BG, SIDEBAR_BG, Some(MUTED));
    // Modal hit scene prevents clicks and Tab traversal into covered controls.
    scene.hits.clear();
    scene.add(r, Action::DiscoveryPanel);
    let x = r.x + 20.0;
    c.text("Settings", x, r.y + 16.0, r.w - 40.0, 17.0, INK, true);
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, r.y + 55.0, r.w - 40.0, 30.0),
        if settings.local_only {
            "Mode: local only — click to enable network browsing"
        } else {
            "Mode: network browsing — click for local only"
        },
        Action::ToggleLocal,
        true,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, r.y + 95.0, r.w - 40.0, 30.0),
        if settings.relays {
            "Relay fallback: on"
        } else {
            "Relay fallback: off"
        },
        Action::ToggleRelay,
        true,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, r.y + 135.0, r.w - 40.0, 30.0),
        "Reset preferences and optional history (retain bookmarks and keys)",
        Action::ResetSettings,
        true,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, r.y + 180.0, r.w - 40.0, 32.0),
        "Save and close",
        Action::FinishSetup,
        true,
    );
    if let Some(error) = error {
        c.text(error, x, r.y + 225.0, r.w - 40.0, 12.0, PLAINTEXT, false);
    }
}

/// A small flyout anchored beside the existing sidebar. It never navigates on
/// receipt or selection; Open homepage is a separate, explicit action.
pub fn discovery_panel(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    discovery: &crate::discovery::Discovery,
) {
    if !discovery.open {
        return;
    }
    let panel = discovery_rect(ui.width, ui.height);
    // Prevent clicks inside the flyout from falling through to a document.
    scene.add(panel, Action::DiscoveryPanel);
    c.round(panel, 10.0, SIDEBAR_BG, SIDEBAR_BG, Some(ACTIVE_TAB));
    let x = panel.x + 14.0;
    let width = panel.w - 28.0;
    let mut y = panel.y + 12.0;
    c.text("Find local nodes", x, y, width - 40.0, 15.0, INK, true);
    let close = Rect::new(panel.x + panel.w - 38.0, y, 25.0, 24.0);
    c.text("Close", close.x - 10.0, y + 2.0, 40.0, 11.0, MUTED, false);
    scene.add(Rect::new(close.x - 10.0, y, 40.0, 24.0), Action::FindLocal);
    y += 30.0;
    let interface = discovery
        .interface
        .and_then(|i| discovery.interfaces.get(i));
    let label = interface.map_or_else(
        || "Choose LAN interface (click to cycle)".into(),
        |i| {
            format!(
                "{} - {} (click to cycle)",
                i.address(),
                i.name
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(40)
                    .collect::<String>()
            )
        },
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, width, 30.0),
        &label,
        Action::SelectLan,
        !discovery.searching() && !discovery.interfaces.is_empty(),
    );
    y += 36.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, width, 28.0),
        "Search selected LAN",
        Action::SearchLan,
        interface.is_some() && !discovery.searching(),
    );
    y += 34.0;
    c.text(&discovery.status, x, y, width, 11.0, MUTED, false);
    y += 22.0;
    c.text(
        "Visible to LAN observers. No anonymity.",
        x,
        y,
        width,
        11.0,
        MUTED,
        false,
    );
    y += 25.0;
    for (index, node) in discovery
        .results
        .iter()
        .enumerate()
        .skip(discovery.page * 4)
        .take(4)
    {
        let row = Rect::new(x, y, width, 35.0);
        let action = Action::LocalNode(index);
        ghost(
            c,
            row,
            discovery.selected == Some(index),
            row.contains(ui.mouse.0, ui.mouse.1) || ui.focus == Some(&action),
        );
        c.text(
            &format!("{}  Untrusted", node.endpoint()),
            x + 7.0,
            y + 1.0,
            width - 14.0,
            12.0,
            INK,
            false,
        );
        c.text(
            &format!("{}...", &node.id().to_string()[..24]),
            x + 7.0,
            y + 19.0,
            width - 14.0,
            10.0,
            MUTED,
            false,
        );
        scene.add(row, action);
        y += 39.0;
    }
    // Fixed footer leaves all result pages and the full selected ID accessible.
    y = panel.y + 344.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, width / 2.0 - 3.0, 25.0),
        "Previous",
        Action::LocalPage(false),
        discovery.page > 0,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + width / 2.0 + 3.0, y, width / 2.0 - 3.0, 25.0),
        "Next",
        Action::LocalPage(true),
        (discovery.page + 1) * 4 < discovery.results.len(),
    );
    y += 34.0;
    if let Some(node) = discovery.selected.and_then(|i| discovery.results.get(i)) {
        c.text(
            "Claimed ID - untrusted until TLS verifies:",
            x,
            y,
            width,
            11.0,
            PLAINTEXT,
            false,
        );
        let id = node.id().to_string();
        // Full ID, split without ellipsis; no trust record is created.
        for (i, part) in id.as_bytes().chunks(31).enumerate() {
            c.text(
                std::str::from_utf8(part).unwrap(),
                x,
                y + 19.0 + i as f32 * 17.0,
                width,
                12.0,
                INK,
                false,
            );
        }
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, y + 59.0, width, 29.0),
            "Open homepage (ynps://)",
            Action::OpenLocal,
            true,
        );
    } else {
        c.text(
            "Select a result to inspect its full node ID.",
            x,
            y,
            width,
            11.0,
            MUTED,
            false,
        );
    }
}

pub fn discovery_rect(width: f32, height: f32) -> Rect {
    Rect::new(
        SIDEBAR_WIDTH + 8.0,
        (height - 486.0).max(8.0),
        (width - SIDEBAR_WIDTH - 24.0).clamp(1.0, 490.0),
        474.0,
    )
}
pub(crate) fn panel_button(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    rect: Rect,
    label: &str,
    action: Action,
    enabled: bool,
) {
    let fill = if enabled && (rect.contains(ui.mouse.0, ui.mouse.1) || ui.focus == Some(&action)) {
        ACTIVE_TAB
    } else {
        HOVER_BG
    };
    c.round(
        rect,
        7.0,
        fill,
        fill,
        Some(if enabled && ui.focus == Some(&action) {
            ACCENT
        } else {
            EDGE
        }),
    );
    c.text(
        label,
        rect.x + 7.0,
        text_y(rect, 12.0),
        rect.w - 14.0,
        12.0,
        if enabled { INK } else { MUTED },
        false,
    );
    if enabled {
        scene.add(rect, action);
    }
}
/// What the connection that delivered the page on screen proved, drawn under
/// the address field. Only TLS to a pinned node ID gets the padlock; plaintext
/// pages say so. Returns the bottom edge of what was drawn.
fn connection_status(c: &mut Canvas<'_>, security: Option<&Security>, left: f32, top: f32) -> f32 {
    let inner = SIDEBAR_WIDTH - 2.0 * PAD;
    let row = |i: f32| Rect::new(left, top + 4.0 + i * 16.0, inner, 16.0);
    match security {
        None | Some(Security::BuiltIn) => top,
        Some(Security::Plaintext) => {
            let r = row(0.0);
            c.text(
                "Not encrypted · not verified",
                r.x + 8.0,
                text_y(r, 10.5),
                inner - 16.0,
                10.5,
                PLAINTEXT,
                false,
            );
            r.y + r.h
        }
        Some(security @ (Security::Verified(_) | Security::Relayed { .. })) => {
            let id = security.destination().expect("verified destination");
            let r = row(0.0);
            c.icon("lock", r.x + 12.0, r.y + r.h / 2.0, VERIFIED);
            c.text(
                if matches!(security, Security::Relayed { .. }) {
                    "Verified node · TLS 1.3 · Relayed"
                } else {
                    "Verified node · TLS 1.3"
                },
                r.x + 22.0,
                text_y(r, 10.5),
                inner - 30.0,
                10.5,
                VERIFIED,
                true,
            );
            // The full node ID, so it can be compared with the one expected,
            // wrapped by measured width so no character is clipped. IDs are ASCII.
            let id = if let Security::Relayed { relay, .. } = security {
                format!("Destination {id} Relay {relay}")
            } else {
                id.to_string()
            };
            let mut rest = id.as_str();
            let mut lines = 0.0;
            while !rest.is_empty() {
                let mut end = rest.len();
                while end > 1
                    && c.text.measure(&rest[..end], 10.5 * c.scale, false) / c.scale > inner - 16.0
                {
                    end -= 1;
                }
                lines += 1.0;
                let r = row(lines);
                c.text(
                    &rest[..end],
                    r.x + 8.0,
                    text_y(r, 10.5),
                    inner - 16.0,
                    10.5,
                    MUTED,
                    false,
                );
                rest = &rest[end..];
            }
            row(lines).y + 16.0
        }
    }
}
fn address_field(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    r: Rect,
    action: Action,
    ui: &ChromeState<'_>,
    focused: bool,
) {
    let fill = if focused { ACTIVE_TAB } else { HOVER_BG };
    c.round(
        r,
        9.0,
        fill,
        fill,
        Some(if focused { ACCENT } else { EDGE }),
    );
    c.icon("search", r.x + 13.0, r.y + r.h / 2.0 + 1.0, MUTED);

    let label = if ui.value.is_empty() {
        match action {
            Action::Find => "Find text…",
            Action::DiscoverField(yami_browser::discover::Field::Search) => {
                "Search saved listings…"
            }
            Action::DiscoverField(yami_browser::discover::Field::Name) => "Directory label",
            Action::DiscoverField(yami_browser::discover::Field::Address) => {
                "yami://<full-node-id>/<key>"
            }
            _ if r.h >= 44.0 => "Enter a YamiNet address…",
            _ => "Enter an address…",
        }
    } else {
        ui.value
    };
    let size = if r.h >= 44.0 { 15.0 } else { 13.0 };
    let x = r.x + 28.0;
    let y = text_y(r, size);
    let text_width = r.w - 34.0;

    let mut visible = label;
    let mut start = 0;
    if focused && !ui.value.is_empty() {
        let caret = ui.caret.min(label.len());
        let mut boundaries: Vec<usize> = label[..caret].char_indices().map(|(i, _)| i).collect();
        boundaries.push(caret);
        let (mut lo, mut hi) = (0, boundaries.len() - 1);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if c.text
                .measure(&label[boundaries[mid]..caret], size * c.scale, false)
                / c.scale
                > text_width - 8.0
            {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        start = boundaries[lo];
        visible = &label[start..];
    }

    if focused {
        if let Some(range) = &ui.selection_range {
            let a = range.start.max(start).min(ui.value.len());
            let b = range.end.max(start).min(ui.value.len());
            if a < b {
                let left = c.text.measure(&ui.value[start..a], size * c.scale, false) / c.scale;
                let right = (c.text.measure(&ui.value[start..b], size * c.scale, false) / c.scale)
                    .min(text_width);
                c.round(
                    Rect::new(x + left, y - 1.0, (right - left).max(0.0), r.h - 14.0),
                    3.0,
                    PLUM_GLOW,
                    PLUM_GLOW,
                    None,
                );
            }
        }
    }
    c.text(
        visible,
        x,
        y,
        text_width,
        size,
        if ui.value.is_empty() { MUTED } else { INK },
        false,
    );

    if focused && !ui.selection {
        let prefix = &ui.value[start.min(ui.value.len())..ui.caret.min(ui.value.len())];
        let cx = x + c.text.measure(prefix, size * c.scale, false) / c.scale;
        c.line(&[(cx, r.y + 8.0), (cx, r.y + r.h - 8.0)], INK, 1.2);
    }
    scene.add(r, action);
}

/// Neutral surface under loading and error states; YLang documents paint their own paper.
pub fn document_viewport(c: &mut Canvas<'_>, _scene: &mut Scene, ui: &ChromeState<'_>) {
    let area = viewport(ui.width, ui.height);
    c.round(area, RADIUS, MAIN_BG, MAIN_BG, None);
}

/// Restores the rounded corners after a document has painted edge to edge.
pub fn round_viewport(c: &mut Canvas<'_>, ui: &ChromeState<'_>) {
    c.mask_corners(viewport(ui.width, ui.height), RADIUS, SIDEBAR_BG);
}

/// Local library flyout. Stored titles never carry a connection indicator.
pub fn library_panel(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    browser: &Browser,
    page: usize,
) {
    let panel = Rect::new(
        SIDEBAR_WIDTH + MARGIN + 8.0,
        MARGIN + 8.0,
        (ui.width - SIDEBAR_WIDTH - 40.0).clamp(200.0, 520.0),
        (ui.height - 32.0).clamp(200.0, 550.0),
    );
    scene.add(panel, Action::DiscoveryPanel);
    c.round(panel, 10.0, SIDEBAR_BG, SIDEBAR_BG, Some(ACTIVE_TAB));
    let x = panel.x + 12.0;
    let w = panel.w - 24.0;
    let mut y = panel.y + 12.0;
    c.text("Bookmarks & local history", x, y, w, 15.0, INK, true);
    y += 28.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, w, 26.0),
        "Save current page (Ctrl+D)",
        Action::SaveBookmark,
        true,
    );
    y += 32.0;
    if let Some(error) = &browser.library_error {
        c.text(error, x, y, w, 11.0, PLAINTEXT, false);
        y += 26.0;
    }
    let Some(library) = &browser.library else {
        c.text("Local storage unavailable", x, y, w, 12.0, MUTED, false);
        return;
    };
    // Two bookmarks per page keeps this usable in compact windows.
    for (index, b) in library
        .bookmarks()
        .iter()
        .enumerate()
        .skip(page * 2)
        .take(2)
    {
        let label = format!("{} / {}", b.folder, b.title);
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, y, w, 25.0),
            &label,
            Action::OpenBookmark(index),
            true,
        );
        y += 27.0;
        for (offset, label, action) in [
            (0.0, "Title", Action::EditBookmark(index, false)),
            (0.34, "Folder", Action::EditBookmark(index, true)),
            (0.68, "Remove", Action::RemoveBookmark(index)),
        ] {
            panel_button(
                c,
                scene,
                ui,
                Rect::new(x + w * offset, y, w * 0.32, 23.0),
                label,
                action,
                true,
            );
        }
        y += 29.0;
    }
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, w * 0.48, 24.0),
        "Previous",
        Action::LibraryPage(false),
        page > 0,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w * 0.52, y, w * 0.48, 24.0),
        "Next",
        Action::LibraryPage(true),
        (page + 1) * 2 < library.bookmarks().len(),
    );
    y += 30.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, w, 24.0),
        "Use current address as Home",
        Action::SetHomepage,
        true,
    );
    y += 30.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, y, w * 0.64, 24.0),
        if library.history_enabled() {
            "History: On (local only)"
        } else {
            "History: Off"
        },
        Action::ToggleHistory,
        true,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w * 0.67, y, w * 0.33, 24.0),
        "Clear history",
        Action::ClearHistory,
        true,
    );
    y += 30.0;
    for (i, v) in library.visits().iter().enumerate().rev().take(3) {
        if y + 25.0 > panel.y + panel.h - 30.0 {
            break;
        }
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, y, w, 24.0),
            &format!("Recent: {}", v.title),
            Action::OpenVisit(i),
            true,
        );
        y += 28.0;
    }
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, panel.y + panel.h - 28.0, w, 22.0),
        "Close",
        Action::Library,
        true,
    );
}

pub fn find_bar(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    find: &yami_browser::find::Find,
    editing: bool,
) {
    let r = Rect::new(
        SIDEBAR_WIDTH + 30.0,
        26.0,
        (ui.width - SIDEBAR_WIDTH - 60.0).min(650.0),
        64.0,
    );
    c.round(r, 8.0, SIDEBAR_BG, SIDEBAR_BG, Some(MUTED));
    scene.add(r, Action::DiscoveryPanel);
    let input = Rect::new(r.x + 10.0, r.y + 8.0, r.w - 245.0, 30.0);
    let find_ui = ChromeState {
        width: ui.width,
        height: ui.height,
        mouse: ui.mouse,
        editing,
        value: &find.query,
        selection: ui.selection,
        selection_range: ui.selection_range.clone(),
        caret: ui.caret,
        focus: ui.focus,
    };
    address_field(c, scene, input, Action::Find, &find_ui, editing);
    c.text(
        &format!(
            "{} / {}",
            if find.matches.is_empty() {
                0
            } else {
                find.current + 1
            },
            find.matches.len()
        ),
        r.x + r.w - 225.0,
        r.y + 15.0,
        75.0,
        12.0,
        INK,
        false,
    );
    for (offset, label, action) in [
        (145.0, "Prev", Action::FindNext(true)),
        (95.0, "Next", Action::FindNext(false)),
        (45.0, "X", Action::CloseFind),
    ] {
        panel_button(
            c,
            scene,
            ui,
            Rect::new(r.x + r.w - offset, r.y + 8.0, 40.0, 30.0),
            label,
            action,
            true,
        );
    }
    c.text(
        "Local exact Unicode search · Enter/Shift+Enter · Esc closes",
        r.x + 10.0,
        r.y + 42.0,
        r.w - 20.0,
        11.0,
        MUTED,
        false,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_layout_hit_areas_are_within_bounds() {
        for (width, height, scale) in [
            (900, 600, 1.0),
            (1440, 900, 1.0),
            (1920, 1080, 1.0),
            (1800, 1200, 2.0),
        ] {
            let mut pix = Pixmap::new(width, height).unwrap();
            let mut text = TextEngine::new();
            let browser = Browser::new();
            let ui = ChromeState {
                width: width as f32 / scale,
                height: height as f32 / scale,
                mouse: (-1.0, -1.0),
                editing: false,
                value: "",
                selection: false,
                selection_range: None,
                caret: 0,
                focus: None,
            };
            let mut c = Canvas::new(&mut pix, &mut text, scale);
            let mut scene = paint(&mut c, &browser, &ui);
            home(&mut c, &mut scene, &ui, &browser);
            for hit in &scene.hits {
                assert!(
                    hit.rect.x >= 0.0
                        && hit.rect.y >= 0.0
                        && hit.rect.x + hit.rect.w <= ui.width
                        && hit.rect.y + hit.rect.h <= ui.height
                );
            }
        }
    }
}
