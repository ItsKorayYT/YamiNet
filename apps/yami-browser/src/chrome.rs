#![allow(clippy::too_many_arguments)]
//! The browser theme is independent of YSL document presentation.
use crate::model::{Browser, PageState, Security};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point,
    RadialGradient, SpreadMode, Stroke, Transform,
};
use yami_render::native::TextEngine;

// Vertical sidebar design color palette
pub const INK: [u8; 4] = [242, 236, 238, 255]; // Primary text #F2ECEE
const MUTED: [u8; 4] = [184, 170, 175, 255]; // Muted text #B8AAAF
const SIDEBAR_BG: [u8; 4] = [33, 27, 29, 255]; // #211B1D
const MAIN_BG: [u8; 4] = [73, 68, 71, 255]; // #494447
const PLUM_GLOW: [u8; 4] = [117, 80, 93, 255]; // #75505D
const HOVER_BG: [u8; 4] = [53, 43, 48, 255]; // #352B30
const ACTIVE_TAB: [u8; 4] = [70, 57, 64, 255]; // #463940
const VERIFIED: [u8; 4] = [156, 204, 170, 255]; // Pinned node proved #9CCCAA
const PLAINTEXT: [u8; 4] = [222, 164, 150, 255]; // No protection #DEA496

pub const SIDEBAR_WIDTH: f32 = 230.0;
pub const MARGIN: f32 = 8.0;
const RADIUS: f32 = 10.0;
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
    fn add(&mut self, rect: Rect, action: Action) {
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
            let mut paint = Paint {
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
    pub caret: usize,
    pub focus: Option<&'a Action>,
}

/// Pinned sites shown as compact tiles under the address field.
const PINS: [(&str, &str); 3] = [
    ("yami", "local:welcome"),
    ("node", "ynp://127.0.0.1:7301/welcome"),
    ("info", "local:about"),
];
const PAD: f32 = 12.0;
const ROW: f32 = 30.0;

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
    let top = Rect::new(left, MARGIN + 6.0, inner, 28.0);
    c.icon("yami", top.x + 9.0, top.y + top.h / 2.0, INK);
    for (i, (action, icon, enabled)) in [
        (Action::Back, "back", browser.active().can_back()),
        (Action::Forward, "forward", browser.active().can_forward()),
        (Action::Reload, "reload", true),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(top.x + top.w - 84.0 + i as f32 * 28.0, top.y, 26.0, 26.0);
        ghost(
            c,
            r,
            false,
            enabled && (hover(r) || ui.focus == Some(&action)),
        );
        let ink = if enabled {
            MUTED
        } else {
            [MUTED[0], MUTED[1], MUTED[2], 90]
        };
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
    let label = Rect::new(left, pins_y + 36.0 + 14.0, inner, 20.0);
    c.text(
        "Tabs",
        label.x + 8.0,
        text_y(label, 11.0),
        inner - 16.0,
        11.0,
        MUTED,
        true,
    );
    let mut y = label.y + label.h + 2.0;
    for (index, tab) in browser.tabs.iter().enumerate() {
        let r = Rect::new(left, y, inner, ROW);
        let active = index == browser.active;
        let focused = ui.focus == Some(&Action::Tab(index));
        ghost(c, r, active, hover(r) || focused);
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
            text_y(r, 12.5),
            r.w - 52.0,
            12.5,
            if active { INK } else { MUTED },
            false,
        );
        scene.add(r, Action::Tab(index));
        // The close control only appears where it is useful, but stays hit-testable
        // so keyboard focus can still reach it.
        let close = Rect::new(r.x + r.w - 24.0, r.y + 7.0, 16.0, 16.0);
        let close_focused = ui.focus == Some(&Action::Close(index));
        if hover(close) || close_focused {
            c.round(close, 4.0, HOVER_BG, HOVER_BG, None);
        }
        if active || hover(r) || close_focused {
            c.icon("close", close.x + 5.0, close.y + 8.0, MUTED);
        }
        scene.add(close, Action::Close(index));
        y += ROW + 2.0;
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

    // Bottom utility: return the current tab to its empty start page.
    let home = Rect::new(left, h - MARGIN - 32.0, 26.0, 26.0);
    ghost(
        c,
        home,
        false,
        hover(home) || ui.focus == Some(&Action::Home),
    );
    c.icon("home", home.x + 13.0, home.y + 13.0, MUTED);
    scene.add(home, Action::Home);

    scene
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
        Some(Security::Verified(id)) => {
            let r = row(0.0);
            c.icon("lock", r.x + 12.0, r.y + r.h / 2.0, VERIFIED);
            c.text(
                "Verified node · TLS 1.3",
                r.x + 22.0,
                text_y(r, 10.5),
                inner - 30.0,
                10.5,
                VERIFIED,
                true,
            );
            // The full node ID, so it can be compared with the one expected,
            // wrapped by measured width so no character is clipped. IDs are ASCII.
            let id = id.to_string();
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
    c.round(r, 8.0, fill, fill, focused.then_some([117, 80, 93, 160]));
    c.icon("search", r.x + 13.0, r.y + r.h / 2.0 + 1.0, MUTED);

    let label = if ui.value.is_empty() {
        "Search or Enter URL…"
    } else {
        ui.value
    };
    let size = 12.5;
    let x = r.x + 26.0;
    let y = text_y(r, size);
    let text_width = r.w - 34.0;

    if focused && ui.selection && !ui.value.is_empty() {
        let tw = (c.text.measure(ui.value, size * c.scale, false) / c.scale).min(text_width);
        c.round(
            Rect::new(x - 1.0, r.y + 7.0, tw + 2.0, r.h - 14.0),
            3.0,
            PLUM_GLOW,
            PLUM_GLOW,
            None,
        );
    }

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

/// The new-tab page: an intentionally empty, muted gradient surface.
pub fn home(c: &mut Canvas<'_>, _scene: &mut Scene, ui: &ChromeState<'_>) {
    let area = viewport(ui.width, ui.height);
    // Base: smoky purple-charcoal at the top left settling into warm grey.
    if let Some(shader) = LinearGradient::new(
        Point::from_xy(area.x, area.y),
        Point::from_xy(area.x + area.w, area.y + area.h),
        vec![
            GradientStop::new(0.0, color([62, 52, 57, 255])),
            GradientStop::new(0.55, color(MAIN_BG)),
            GradientStop::new(1.0, color([66, 63, 65, 255])),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    ) {
        c.fill_shader(area, RADIUS, shader);
    }
    // Two diffuse glows: dusty rose low-left, plum toward the centre.
    let reach = area.w.max(area.h);
    c.glow(
        area,
        RADIUS,
        area.x + area.w * 0.32,
        area.y + area.h * 0.62,
        reach * 0.42,
        [150, 98, 114, 110],
    );
    c.glow(
        area,
        RADIUS,
        area.x + area.w * 0.55,
        area.y + area.h * 0.40,
        reach * 0.38,
        [PLUM_GLOW[0], PLUM_GLOW[1], PLUM_GLOW[2], 70],
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sidebar_layout_hit_areas_are_within_bounds() {
        for (width, height, scale) in [(1440, 900, 1.0), (1920, 1080, 1.0), (2880, 1800, 2.0)] {
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
                caret: 0,
                focus: None,
            };
            let mut c = Canvas::new(&mut pix, &mut text, scale);
            let mut scene = paint(&mut c, &browser, &ui);
            home(&mut c, &mut scene, &ui);
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
