#![allow(clippy::too_many_arguments)]
//! The browser theme is independent of YSL document presentation.
use crate::model::{Browser, PageState};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, SpreadMode,
    Stroke, Transform,
};
use yami_render::native::TextEngine;

pub const INK: [u8; 4] = [23, 71, 107, 255];
const WHITE: [u8; 4] = [234, 251, 255, 255];
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
    HomeAddress,
    Go,
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
    clip: Option<tiny_skia::Mask>,
}
impl<'a> Canvas<'a> {
    pub fn new(pix: &'a mut Pixmap, text: &'a mut TextEngine, scale: f32) -> Self {
        Self {
            pix,
            text,
            scale,
            clip: None,
        }
    }
    fn clip_viewport(&mut self, r: Rect) {
        let mut path = PathBuilder::new();
        let z = 17.0;
        path.move_to(r.x + z, r.y);
        path.line_to(r.x + r.w - z, r.y);
        path.quad_to(r.x + r.w, r.y, r.x + r.w, r.y + z);
        path.line_to(r.x + r.w, r.y + r.h - z);
        path.quad_to(r.x + r.w, r.y + r.h, r.x + r.w - z, r.y + r.h);
        path.line_to(r.x + z, r.y + r.h);
        path.quad_to(r.x, r.y + r.h, r.x, r.y + r.h - z);
        path.line_to(r.x, r.y + z);
        path.quad_to(r.x, r.y, r.x + z, r.y);
        path.close();
        if let (Some(path), Some(mut mask)) = (
            path.finish(),
            tiny_skia::Mask::new(self.pix.width(), self.pix.height()),
        ) {
            mask.fill_path(
                &path,
                FillRule::Winding,
                true,
                Transform::from_scale(self.scale, self.scale),
            );
            self.clip = Some(mask);
        }
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
        if let Some(path) = path.finish() {
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
                self.clip.as_ref(),
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
                    self.clip.as_ref(),
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
                self.clip.as_ref(),
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
                self.clip.as_ref(),
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
    pub fn shadow(&mut self, r: Rect, radius: f32) {
        for (spread, alpha) in [(8.0, 4), (5.0, 7), (2.0, 12)] {
            self.round(
                Rect::new(
                    r.x - spread,
                    r.y - spread + 4.0,
                    r.w + 2.0 * spread,
                    r.h + 2.0 * spread,
                ),
                radius + spread,
                [0, 73, 135, alpha],
                [0, 73, 135, alpha],
                None,
            );
        }
    }
    pub fn orb(&mut self, x: f32, y: f32, r: f32) {
        self.oval(
            Rect::new(x - r, y - r, r * 2.0, r * 2.0),
            [168, 237, 255, 255],
            [8, 120, 204, 255],
        );
        self.oval(
            Rect::new(x - r * 0.65, y - r * 0.8, r * 1.2, r * 0.7),
            [255, 255, 255, 210],
            [255, 255, 255, 10],
        );
        self.line(
            &[
                (x - r * 1.35, y + r * 0.3),
                (x - r * 0.7, y + r * 0.8),
                (x + r * 0.75, y - r * 0.6),
                (x + r * 1.3, y - r * 0.3),
            ],
            [190, 255, 248, 220],
            2.0,
        );
        self.oval(
            Rect::new(x + r * 0.1, y - r * 0.2, r * 0.55, r * 0.8),
            [53, 215, 205, 220],
            [145, 214, 160, 150],
        );
    }
    pub fn bubble(&mut self, x: f32, y: f32, r: f32) {
        self.oval(
            Rect::new(x - r, y - r, 2.0 * r, 2.0 * r),
            [234, 251, 255, 95],
            [53, 215, 205, 8],
        );
        self.oval(
            Rect::new(x - r * 0.76, y - r * 0.86, r * 1.45, r * 0.68),
            [255, 255, 255, 120],
            [255, 255, 255, 0],
        );
        let mut points = Vec::new();
        for i in 0..=64 {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            points.push((x + r * a.cos(), y + r * a.sin()));
        }
        self.line(&points, [234, 251, 255, 160], 1.2);
        self.oval(
            Rect::new(x - r * 0.56, y - r * 0.65, r * 0.2, r * 0.1),
            [255, 255, 255, 220],
            [255, 255, 255, 10],
        );
    }
    pub fn sweep(&mut self, x: f32, y: f32, w: f32) {
        let mut path = PathBuilder::new();
        path.move_to(x, y + 30.0);
        path.cubic_to(
            x + w * 0.05,
            y - 30.0,
            x + w * 0.98,
            y - 35.0,
            x + w,
            y + 4.0,
        );
        if let Some(path) = path.finish() {
            let mut p = Paint::default();
            p.set_color(color([234, 251, 255, 105]));
            self.pix.stroke_path(
                &path,
                &p,
                &Stroke {
                    width: 1.2,
                    ..Stroke::default()
                },
                Transform::from_scale(self.scale, self.scale),
                self.clip.as_ref(),
            );
        }
    }
    pub fn icon(&mut self, kind: &str, x: f32, y: f32, ink: [u8; 4]) {
        match kind {
            "back" => {
                self.line(&[(x + 6.0, y - 6.0), (x, y), (x + 6.0, y + 6.0)], ink, 2.0);
                self.line(&[(x, y), (x + 14.0, y)], ink, 2.0);
            }
            "forward" => {
                self.line(
                    &[(x + 8.0, y - 6.0), (x + 14.0, y), (x + 8.0, y + 6.0)],
                    ink,
                    2.0,
                );
                self.line(&[(x, y), (x + 14.0, y)], ink, 2.0);
            }
            "close" => {
                self.line(&[(x, y - 4.0), (x + 8.0, y + 4.0)], ink, 1.5);
                self.line(&[(x + 8.0, y - 4.0), (x, y + 4.0)], ink, 1.5);
            }
            "plus" => {
                self.line(&[(x, y), (x + 12.0, y)], ink, 1.8);
                self.line(&[(x + 6.0, y - 6.0), (x + 6.0, y + 6.0)], ink, 1.8);
            }
            "home" => {
                self.line(
                    &[
                        (x, y),
                        (x + 7.0, y - 7.0),
                        (x + 14.0, y),
                        (x + 14.0, y + 8.0),
                        (x + 1.0, y + 8.0),
                        (x + 1.0, y),
                    ],
                    ink,
                    1.8,
                );
            }
            "reload" => {
                self.line(
                    &[
                        (x + 13.0, y - 3.0),
                        (x + 9.0, y - 7.0),
                        (x + 3.0, y - 6.0),
                        (x, y - 1.0),
                        (x + 2.0, y + 5.0),
                        (x + 8.0, y + 7.0),
                        (x + 13.0, y + 3.0),
                    ],
                    ink,
                    1.8,
                );
                self.line(
                    &[(x + 13.0, y - 8.0), (x + 13.0, y - 2.0), (x + 7.0, y - 2.0)],
                    ink,
                    1.8,
                );
            }
            "page" => {
                self.round(
                    Rect::new(x, y - 8.0, 14.0, 18.0),
                    3.0,
                    [255, 255, 255, 0],
                    [255, 255, 255, 0],
                    Some(ink),
                );
                for dy in [-3.0, 2.0, 6.0] {
                    self.line(&[(x + 4.0, y + dy), (x + 10.0, y + dy)], ink, 1.4);
                }
            }
            "star" => {
                self.line(
                    &[
                        (x + 7.0, y - 9.0),
                        (x + 10.0, y - 3.0),
                        (x + 17.0, y - 2.0),
                        (x + 12.0, y + 3.0),
                        (x + 13.0, y + 10.0),
                        (x + 7.0, y + 7.0),
                        (x + 1.0, y + 10.0),
                        (x + 2.0, y + 3.0),
                        (x - 3.0, y - 2.0),
                        (x + 4.0, y - 3.0),
                        (x + 7.0, y - 9.0),
                    ],
                    ink,
                    1.6,
                );
            }
            _ => self.orb(x + 7.0, y, 9.0),
        }
    }
}
fn color(rgba: [u8; 4]) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}

pub struct ChromeState<'a> {
    pub width: f32,
    pub height: f32,
    pub mouse: (f32, f32),
    pub editing: bool,
    pub home_editing: bool,
    pub value: &'a str,
    pub selection: bool,
    pub caret: usize,
    pub focus: Option<&'a Action>,
}
fn button(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    r: Rect,
    action: Action,
    icon: &str,
    enabled: bool,
    ui: &ChromeState<'_>,
) {
    let hovered = enabled && r.contains(ui.mouse.0, ui.mouse.1);
    if hovered || ui.focus == Some(&action) {
        c.round(
            r,
            10.0,
            [255, 255, 255, 190],
            [168, 237, 255, 110],
            Some([255, 255, 255, 220]),
        );
    }
    c.icon(
        icon,
        r.x + (r.w - 14.0) / 2.0,
        r.y + r.h / 2.0,
        if enabled { INK } else { [90, 147, 181, 160] },
    );
    if enabled {
        scene.add(r, action);
    }
}
pub fn paint(c: &mut Canvas<'_>, browser: &Browser, ui: &ChromeState<'_>) -> Scene {
    let mut scene = Scene::default();
    let w = ui.width;
    let h = ui.height;
    c.round(
        Rect::new(0.0, 0.0, w, h),
        0.0,
        [41, 197, 246, 255],
        [234, 251, 255, 255],
        None,
    );
    c.round(
        Rect::new(0.0, 0.0, w, 108.0),
        0.0,
        [91, 210, 250, 255],
        [166, 231, 251, 255],
        None,
    );
    c.round(
        Rect::new(0.0, 0.0, w, 1.0),
        0.0,
        [255, 255, 255, 220],
        [255, 255, 255, 220],
        None,
    );
    c.round(
        Rect::new(0.0, 0.0, w, 24.0),
        0.0,
        [234, 251, 255, 64],
        [234, 251, 255, 0],
        None,
    );
    c.orb(28.0, 30.0, 12.0);
    let available = w - 100.0;
    let tab_width = (available / browser.tabs.len().max(1) as f32).clamp(45.0, 218.0);
    for (index, tab) in browser.tabs.iter().enumerate() {
        let r = Rect::new(53.0 + index as f32 * tab_width, 11.0, tab_width - 5.0, 38.0);
        let active = index == browser.active;
        let hover = r.contains(ui.mouse.0, ui.mouse.1);
        c.round(
            r,
            12.0,
            if active {
                [245, 254, 255, 238]
            } else {
                [230, 253, 255, if hover { 150 } else { 85 }]
            },
            if active {
                [190, 239, 255, 235]
            } else {
                [126, 211, 242, 110]
            },
            Some([255, 255, 255, if active { 240 } else { 120 }]),
        );
        scene.add(r, Action::Tab(index));
        c.oval(
            Rect::new(r.x + 13.0, r.y + 14.0, 8.0, 8.0),
            if matches!(tab.state, PageState::Loading) {
                [53, 215, 205, 255]
            } else {
                [8, 120, 204, 255]
            },
            [41, 197, 246, 255],
        );
        if tab_width > 90.0 {
            c.text(
                tab.title(),
                r.x + 28.0,
                r.y + 9.0,
                r.w - 56.0,
                12.0,
                INK,
                active,
            );
        }
        let close = Rect::new(r.x + r.w - 27.0, r.y + 7.0, 23.0, 24.0);
        if close.contains(ui.mouse.0, ui.mouse.1) {
            c.round(close, 7.0, [255, 255, 255, 200], [255, 255, 255, 130], None);
        }
        c.icon("close", close.x + 7.0, close.y + 12.0, INK);
        scene.add(close, Action::Close(index));
    }
    let new = Rect::new(
        (53.0 + browser.tabs.len() as f32 * tab_width).min(w - 41.0),
        14.0,
        30.0,
        30.0,
    );
    button(
        c,
        &mut scene,
        new,
        Action::NewTab,
        "plus",
        browser.tabs.len() < 12,
        ui,
    );
    c.round(
        Rect::new(8.0, 53.0, w - 16.0, 49.0),
        16.0,
        [243, 253, 255, 165],
        [166, 228, 250, 160],
        Some([255, 255, 255, 210]),
    );
    for (x, action, icon, enabled) in [
        (15.0, Action::Back, "back", browser.active().can_back()),
        (
            51.0,
            Action::Forward,
            "forward",
            browser.active().can_forward(),
        ),
        (87.0, Action::Reload, "reload", true),
        (123.0, Action::Home, "home", true),
    ] {
        button(
            c,
            &mut scene,
            Rect::new(x, 60.0, 32.0, 34.0),
            action,
            icon,
            enabled,
            ui,
        );
    }
    let address = Rect::new(167.0, 60.0, w - 219.0, 34.0);
    address_field(
        c,
        &mut scene,
        address,
        Action::Address,
        ui,
        ui.editing,
        false,
    );
    button(
        c,
        &mut scene,
        Rect::new(w - 46.0, 60.0, 30.0, 34.0),
        Action::Go,
        "forward",
        true,
        ui,
    );
    scene
}
pub fn footer(c: &mut Canvas<'_>, browser: &Browser, ui: &ChromeState<'_>) {
    let w = ui.width;
    let h = ui.height;
    let status = match &browser.active().state {
        PageState::Home => "Local start page  •  No background connections",
        PageState::Loading => "Retrieving document over localhost YNP…",
        PageState::Ready(_) => "YLang + YSL  •  Native rendering",
        PageState::Error(_) => "Navigation failed  •  Check the address and local node",
    };
    c.round(
        Rect::new(0.0, h - 24.0, w, 24.0),
        0.0,
        [234, 251, 255, 255],
        [211, 242, 252, 255],
        Some([255, 255, 255, 150]),
    );
    c.oval(
        Rect::new(14.0, h - 15.0, 6.0, 6.0),
        [53, 180, 169, 255],
        [53, 180, 169, 255],
    );
    c.text(status, 27.0, h - 20.0, w - 325.0, 11.0, INK, false);
    c.text(
        "Experimental · not secure for sensitive use",
        w - 296.0,
        h - 20.0,
        286.0,
        10.5,
        INK,
        false,
    );
}
fn address_field(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    r: Rect,
    action: Action,
    ui: &ChromeState<'_>,
    focused: bool,
    home: bool,
) {
    if focused {
        c.round(
            Rect::new(r.x - 3.0, r.y - 3.0, r.w + 6.0, r.h + 6.0),
            r.h / 2.0,
            [41, 197, 246, 90],
            [41, 197, 246, 90],
            Some([234, 251, 255, 230]),
        );
    }
    c.round(
        r,
        r.h / 2.0,
        [255, 255, 255, 235],
        [234, 251, 255, 220],
        Some(if focused {
            [8, 120, 204, 210]
        } else {
            [255, 255, 255, 240]
        }),
    );
    c.orb(r.x + 21.0, r.y + r.h / 2.0, if home { 10.0 } else { 8.0 });
    let label = if ui.value.is_empty() {
        if home {
            "Enter a localhost YNP address"
        } else {
            "Enter ynp://127.0.0.1:7301/welcome"
        }
    } else {
        ui.value
    };
    let x = r.x + 42.0;
    let y = r.y + (r.h - 21.0) / 2.0;
    let size = if home { 16.0 } else { 13.0 };
    let text_width = r.w - if home { 100.0 } else { 57.0 };
    if focused && ui.selection && !ui.value.is_empty() {
        let tw = (c.text.measure(ui.value, size * c.scale, false) / c.scale).min(text_width);
        c.round(
            Rect::new(x, y, tw, 22.0),
            3.0,
            [168, 237, 255, 255],
            [168, 237, 255, 255],
            None,
        );
    }
    // Long editable addresses scroll horizontally to keep the caret visible.
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
        if ui.value.is_empty() {
            [69, 120, 155, 255]
        } else {
            INK
        },
        false,
    );
    if focused && !ui.selection {
        let prefix = &ui.value[start.min(ui.value.len())..ui.caret.min(ui.value.len())];
        let cx = x + c.text.measure(prefix, size * c.scale, false) / c.scale;
        c.line(&[(cx, y + 3.0), (cx, y + 19.0)], [8, 120, 204, 255], 1.3);
    }
    scene.add(r, action);
}

pub fn home(c: &mut Canvas<'_>, scene: &mut Scene, ui: &ChromeState<'_>) {
    let w = ui.width;
    let bottom = ui.height - 28.0;
    let top = 108.0;
    let area = Rect::new(8.0, top, w - 16.0, bottom - top);
    c.clip_viewport(area);
    c.round(
        area,
        17.0,
        [8, 120, 204, 255],
        [168, 237, 255, 255],
        Some([234, 251, 255, 220]),
    );
    // Original procedural scenery, not the reference image: sky, clouds, a
    // distant skyline and reflective water. Every element is bundled code.
    let horizon = (bottom - 100.0).max(top + 275.0);
    c.sweep(w / 2.0 - 242.0, top + 70.0, 480.0);
    c.bubble(w * 0.09, top + 232.0, 40.0);
    c.bubble(w * 0.91, top + 304.0, 55.0);
    c.bubble(w * 0.82, top + 84.0, 15.0);

    for (x, y, s) in [
        (w * 0.06, top + 120.0, 1.0),
        (w * 0.75, top + 170.0, 1.3),
        (w * 0.87, top + 55.0, 0.8),
        (w * 0.15, horizon - 15.0, 0.8),
    ] {
        for (dx, dy, r) in [
            (0.0, 10.0, 40.0),
            (35.0, -2.0, 48.0),
            (77.0, 12.0, 34.0),
            (105.0, 25.0, 25.0),
        ] {
            c.oval(
                Rect::new(x + dx * s, y + dy * s, r * s, r * 0.6 * s),
                [255, 255, 255, 100],
                [255, 255, 255, 15],
            );
        }
    }
    c.round(
        Rect::new(9.0, horizon, w - 18.0, (bottom - horizon).max(1.0)),
        10.0,
        [117, 220, 238, 200],
        [203, 248, 247, 255],
        None,
    );
    let city_base = horizon + 10.0;
    for i in 0..16 {
        let x = w * 0.65 + i as f32 * 18.0;
        let height = 26.0 + ((i * 23) % 71) as f32;
        c.round(
            Rect::new(x, city_base - height, 10.0, height),
            3.0,
            [242, 255, 255, 160],
            [105, 193, 216, 120],
            None,
        );
        c.line(
            &[
                (x + 3.0, city_base - height + 7.0),
                (x + 3.0, city_base - 4.0),
            ],
            [255, 255, 255, 150],
            1.2,
        );
    }
    for i in 0..8 {
        let y = horizon + 15.0 + i as f32 * 11.0;
        c.line(
            &[
                (w * 0.1 + (i % 3) as f32 * 17.0, y),
                (w * 0.9 - i as f32 * 11.0, y),
            ],
            [255, 255, 255, 60],
            1.0,
        );
    }
    c.oval(
        Rect::new(-w * 0.14, bottom - 61.0, w * 0.55, 125.0),
        [145, 214, 160, 210],
        [53, 153, 149, 220],
    );
    c.oval(
        Rect::new(w * 0.77, bottom - 41.0, w * 0.4, 100.0),
        [160, 232, 184, 210],
        [53, 173, 169, 210],
    );
    c.text("YAMI BROWSER", 30.0, top + 19.0, 170.0, 10.5, WHITE, true);
    c.text(
        "THE NATIVE DOCUMENT WEB",
        30.0,
        top + 36.0,
        250.0,
        10.0,
        [207, 246, 255, 255],
        false,
    );
    let pill = Rect::new(w - 216.0, top + 20.0, 185.0, 27.0);
    c.round(
        pill,
        14.0,
        [255, 255, 255, 70],
        [255, 255, 255, 25],
        Some([255, 255, 255, 130]),
    );
    c.text(
        "LOCAL FIRST · EXPERIMENTAL",
        pill.x + 14.0,
        pill.y + 5.0,
        164.0,
        10.0,
        WHITE,
        true,
    );
    let hero_y = top + 83.0;
    c.orb(w / 2.0 - 154.0, hero_y + 40.0, 28.0);
    c.text(
        "YamiNet",
        w / 2.0 - 112.0,
        hero_y - 3.0,
        355.0,
        64.0,
        [5, 107, 172, 110],
        true,
    );
    c.text(
        "YamiNet",
        w / 2.0 - 112.0,
        hero_y - 5.0,
        355.0,
        64.0,
        WHITE,
        true,
    );
    c.line(
        &[
            (w / 2.0 + 144.0, hero_y + 4.0),
            (w / 2.0 + 144.0, hero_y + 21.0),
        ],
        [255, 255, 255, 230],
        1.5,
    );
    c.line(
        &[
            (w / 2.0 + 136.0, hero_y + 12.0),
            (w / 2.0 + 152.0, hero_y + 12.0),
        ],
        [255, 255, 255, 230],
        1.5,
    );
    c.text(
        "A brighter kind of web.",
        w / 2.0 - 104.0,
        hero_y + 78.0,
        300.0,
        17.0,
        WHITE,
        false,
    );
    let field = Rect::new(
        (w - 600.0).max(58.0) / 2.0,
        hero_y + 127.0,
        (w - 66.0).min(600.0),
        57.0,
    );
    c.shadow(field, 29.0);
    address_field(
        c,
        scene,
        field,
        Action::HomeAddress,
        ui,
        ui.home_editing,
        true,
    );
    let go = Rect::new(field.x + field.w - 48.0, field.y + 8.0, 40.0, 41.0);
    c.round(
        go,
        21.0,
        [89, 222, 248, 255],
        [8, 151, 222, 255],
        Some([255, 255, 255, 240]),
    );
    c.icon("forward", go.x + 13.0, go.y + 20.0, WHITE);
    scene.add(go, Action::Go);
    let gap = 12.0;
    let tiles_width = (w - 80.0).min(600.0);
    let tw = (tiles_width - 3.0 * gap) / 4.0;
    for (index, (title, detail, icon, url)) in [
        ("Welcome", "Bundled page", "home", "local:welcome"),
        ("About YamiNet", "Bundled page", "page", "local:about"),
        (
            "Node one",
            "127.0.0.1:7301",
            "star",
            "ynp://127.0.0.1:7301/welcome",
        ),
        (
            "Node two",
            "127.0.0.1:7302",
            "star",
            "ynp://127.0.0.1:7302/about",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let r = Rect::new(
            (w - tiles_width) / 2.0 + index as f32 * (tw + gap),
            field.y + 83.0,
            tw,
            96.0,
        );
        let hover = r.contains(ui.mouse.0, ui.mouse.1) || ui.focus == Some(&Action::Navigate(url));
        c.shadow(r, 17.0);
        c.round(
            r,
            17.0,
            if hover {
                [255, 255, 255, 245]
            } else {
                [234, 251, 255, 210]
            },
            [200, 241, 253, if hover { 240 } else { 190 }],
            Some([255, 255, 255, 240]),
        );
        c.icon(icon, r.x + r.w / 2.0 - 7.0, r.y + 24.0, [8, 120, 204, 255]);
        c.text(title, r.x + 12.0, r.y + 46.0, r.w - 24.0, 13.0, INK, true);
        c.text(
            detail,
            r.x + 12.0,
            r.y + 68.0,
            r.w - 24.0,
            10.0,
            [52, 110, 147, 255],
            false,
        );
        scene.add(r, Action::Navigate(url));
    }
    if bottom > field.y + 257.0 {
        let r = Rect::new((w - 600.0) / 2.0, bottom - 63.0, 600.0, 40.0);
        c.round(
            r,
            20.0,
            [234, 251, 255, 190],
            [234, 251, 255, 130],
            Some([255, 255, 255, 220]),
        );
        c.oval(
            Rect::new(r.x + 17.0, r.y + 16.0, 8.0, 8.0),
            [53, 180, 169, 255],
            [53, 180, 169, 255],
        );
        c.text(
            "Your documents. Your window to something new.",
            r.x + 35.0,
            r.y + 10.0,
            535.0,
            12.0,
            INK,
            false,
        );
    }
    c.clip = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resized_home_controls_remain_inside_window_and_hit_correctly() {
        for (width, height, scale) in [(720, 540, 1.0), (1180, 800, 1.0), (1440, 1080, 2.0)] {
            let mut pix = Pixmap::new(width, height).unwrap();
            let mut text = TextEngine::new();
            let browser = Browser::new();
            let ui = ChromeState {
                width: width as f32 / scale,
                height: height as f32 / scale,
                mouse: (-1.0, -1.0),
                editing: false,
                home_editing: false,
                value: "",
                selection: false,
                caret: 0,
                focus: None,
            };
            let mut c = Canvas::new(&mut pix, &mut text, scale);
            let mut scene = paint(&mut c, &browser, &ui);
            home(&mut c, &mut scene, &ui);
            footer(&mut c, &browser, &ui);
            for hit in &scene.hits {
                assert!(
                    hit.rect.x >= 0.0
                        && hit.rect.y >= 0.0
                        && hit.rect.x + hit.rect.w <= ui.width
                        && hit.rect.y + hit.rect.h <= ui.height
                );
            }
            let field = scene
                .hits
                .iter()
                .find(|hit| hit.action == Action::HomeAddress)
                .unwrap();
            assert_eq!(
                scene.hit(field.rect.x + 50.0, field.rect.y + 20.0),
                Some(Action::HomeAddress)
            );
            assert!(scene
                .hits
                .iter()
                .any(|hit| hit.action == Action::Navigate("local:welcome")));
        }
    }
    #[test]
    fn tab_close_has_priority_and_long_unicode_address_is_bounded() {
        let mut pix = Pixmap::new(1180, 800).unwrap();
        let mut text = TextEngine::new();
        let mut browser = Browser::new();
        for _ in 1..12 {
            browser.open_tab();
        }
        let value = "é".repeat(2048);
        let ui = ChromeState {
            width: 1180.0,
            height: 800.0,
            mouse: (-1.0, -1.0),
            editing: true,
            home_editing: false,
            value: &value,
            selection: false,
            caret: value.len(),
            focus: None,
        };
        let mut c = Canvas::new(&mut pix, &mut text, 1.0);
        let scene = paint(&mut c, &browser, &ui);
        assert!(!scene.hits.iter().any(|hit| hit.action == Action::NewTab));
        let close = scene
            .hits
            .iter()
            .find(|hit| hit.action == Action::Close(4))
            .unwrap();
        assert_eq!(
            scene.hit(close.rect.x + 5.0, close.rect.y + 5.0),
            Some(Action::Close(4))
        );
    }
}
