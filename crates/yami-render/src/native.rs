//! Antialiased, shaped native text and responsive document layout.
//!
//! The browser chrome uses `TextEngine`, while website presentation comes only
//! from its YSL sheet. This module neither loads resources nor performs I/O.
use std::collections::VecDeque;
mod rich;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap};
use tiny_skia::{Paint, Pixmap, PremultipliedColorU8, Rect, Transform};
use ylang::{Document, Node, NodeKind};
use ysl::{Color, Stylesheet, TextStyle};

use crate::{RenderError, MAX_ROWS};

const UI_CACHE_SIZE: usize = 128;
const MAX_LINE_BYTES: usize = 4096;
const MAX_BLOCKS: usize = 8192;
const MAX_TEXT_BYTES: usize = ylang::MAX_SOURCE;

/// A text rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl TextRect {
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

struct CachedText {
    text: String,
    width: f32,
    size: f32,
    bold: bool,
    buffer: Buffer,
}

/// Shared font database, antialiased glyph cache and bounded UI shaping cache.
/// Open Sans is bundled, including its license, so Windows needs no font install.
pub struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
    ui_cache: VecDeque<CachedText>,
}
impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}
impl TextEngine {
    pub fn new() -> Self {
        let mut database = cosmic_text::fontdb::Database::new();
        database.load_font_data(include_bytes!("../assets/fonts/OpenSans-Regular.ttf").to_vec());
        database.load_font_data(include_bytes!("../assets/fonts/OpenSans-Semibold.ttf").to_vec());
        database.load_font_data(include_bytes!("../assets/fonts/DejaVuSans-Oblique.ttf").to_vec());
        database.load_font_data(include_bytes!("../assets/fonts/DejaVuSansMono.ttf").to_vec());
        database.set_monospace_family("DejaVu Sans Mono");
        database.set_sans_serif_family("Open Sans");
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), database),
            swash: SwashCache::new(),
            ui_cache: VecDeque::new(),
        }
    }

    fn shape(&mut self, text: &str, width: f32, size: f32, bold: bool, wrap: Wrap) -> Buffer {
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * 1.5));
        buffer.set_size(&mut self.fonts, Some(width), None);
        buffer.set_wrap(&mut self.fonts, wrap);
        let attrs = Attrs::new()
            .family(Family::Name("Open Sans"))
            .weight(Weight(if bold { 600 } else { 400 }));
        buffer.set_text(&mut self.fonts, text, &attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.fonts, false);
        buffer
    }

    fn trim_raster_cache(&mut self) {
        if self.swash.image_cache.len() > 4096 {
            self.swash = SwashCache::new();
        }
    }

    fn cache_ui(&mut self, text: &str, width: f32, size: f32, bold: bool) {
        self.trim_raster_cache();
        let mut limit = text.len().min(MAX_LINE_BYTES);
        while !text.is_char_boundary(limit) {
            limit -= 1;
        }
        let text = &text[..limit];
        if let Some(index) = self.ui_cache.iter().position(|entry| {
            entry.text == text && entry.width == width && entry.size == size && entry.bold == bold
        }) {
            let entry = self.ui_cache.remove(index).expect("existing cache index");
            self.ui_cache.push_front(entry);
            return;
        }
        let buffer = self.shape(text, width, size, bold, Wrap::None);
        if self.ui_cache.len() == UI_CACHE_SIZE {
            self.ui_cache.pop_back();
        }
        self.ui_cache.push_front(CachedText {
            text: text.to_owned(),
            width,
            size,
            bold,
            buffer,
        });
    }

    /// Draw one line of chrome text, clipped to its width and the framebuffer.
    /// All coordinates and sizes are physical pixels; scale at the call site.
    /// UI text is limited to a valid UTF-8 prefix of at most 4096 bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        color: Color,
    ) {
        self.draw_style(
            pixmap,
            text,
            TextRect {
                x,
                y,
                width,
                height: size * 1.6,
            },
            size,
            color,
            false,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_bold(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        size: f32,
        color: Color,
    ) {
        self.draw_style(
            pixmap,
            text,
            TextRect {
                x,
                y,
                width,
                height: size * 1.6,
            },
            size,
            color,
            true,
        );
    }

    fn draw_style(
        &mut self,
        pixmap: &mut Pixmap,
        text: &str,
        clip: TextRect,
        size: f32,
        color: Color,
        bold: bool,
    ) {
        if !valid_rect(clip) || !size.is_finite() || !(1.0..=512.0).contains(&size) {
            return;
        }
        self.cache_ui(text, clip.width, size, bold);
        let Some(entry) = self.ui_cache.front() else {
            return;
        };
        paint_buffer(
            &entry.buffer,
            &mut self.fonts,
            &mut self.swash,
            pixmap,
            clip.x,
            clip.y,
            color,
            clip,
        );
    }

    /// Measure the same bounded UTF-8 prefix used by chrome drawing.
    pub fn measure(&mut self, text: &str, size: f32, bold: bool) -> f32 {
        if !size.is_finite() || !(1.0..=512.0).contains(&size) {
            return 0.0;
        }
        self.cache_ui(text, 32768.0, size, bold);
        self.ui_cache
            .front()
            .map(|entry| {
                entry
                    .buffer
                    .layout_runs()
                    .map(|run| run.line_w)
                    .fold(0.0, f32::max)
            })
            .unwrap_or(0.0)
    }
}

fn valid_rect(rect: TextRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.width > 0.0
        && rect.height > 0.0
}

#[allow(clippy::too_many_arguments)]
fn paint_buffer(
    buffer: &Buffer,
    fonts: &mut FontSystem,
    swash: &mut SwashCache,
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    color: Color,
    clip: TextRect,
) {
    let Color(r, g, b) = color;
    let x = x.round() as i32;
    let y = y.round() as i32;
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let left = (clip.x.floor() as i32).clamp(0, width);
    let top = (clip.y.floor() as i32).clamp(0, height);
    let right = ((clip.x + clip.width).ceil() as i32).clamp(0, width);
    let bottom = ((clip.y + clip.height).ceil() as i32).clamp(0, height);
    let pixels = pixmap.pixels_mut();
    let default_ink = cosmic_text::Color::rgb(r, g, b);
    // Cull before asking Swash to rasterize: a long clipped address or a
    // wrapped paragraph must not rasterize thousands of invisible glyphs.
    for run in buffer.layout_runs() {
        let run_top = y as f32 + run.line_top;
        let margin = run.line_height;
        if run_top + run.line_height + margin < top as f32 || run_top - margin > bottom as f32 {
            continue;
        }
        for glyph in run.glyphs {
            let glyph_left = x as f32 + glyph.x;
            if glyph_left + glyph.w + margin < left as f32 || glyph_left - margin > right as f32 {
                continue;
            }
            let physical = glyph.physical((0.0, 0.0), 1.0);
            swash.with_pixels(
                fonts,
                physical.cache_key,
                glyph.color_opt.unwrap_or(default_ink),
                |gx, gy, ink| {
                    let px = x + physical.x + gx;
                    let py = y + run.line_y as i32 + physical.y + gy;
                    if px < left || px >= right || py < top || py >= bottom {
                        return;
                    }
                    let index = py as usize * width as usize + px as usize;
                    let previous = pixels[index];
                    let alpha = u16::from(ink.a());
                    let inv = 255 - alpha;
                    let blend = |source: u8, dest: u8| {
                        ((u16::from(source) * alpha + u16::from(dest) * inv + 127) / 255) as u8
                    };
                    let a = (alpha + (u16::from(previous.alpha()) * inv + 127) / 255) as u8;
                    pixels[index] = PremultipliedColorU8::from_rgba(
                        blend(ink.r(), previous.red()),
                        blend(ink.g(), previous.green()),
                        blend(ink.b(), previous.blue()),
                        a,
                    )
                    .expect("valid source-over premultiplied color");
                },
            );
        }
    }
}

struct Block {
    buffer: Buffer,
    x: f32,
    y: f32,
    height: f32,
    color: Color,
    links: Vec<TextRect>,
    target: Option<String>,
}

/// A shaped display list for one document at a particular viewport width/DPI.
/// It is rebuilt only when the document, dimensions or pixel scale changes.
pub struct NativeDocument {
    pub width: f32,
    pub height: f32,
    pub paper: Color,
    scale: f32,
    blocks: Vec<Block>,
    decorations: Vec<rich::Decoration>,
    images: Vec<rich::ImageBlock>,
    link_states: Option<(Color, Color)>,
}
impl NativeDocument {
    pub fn layout(
        document: &Document,
        style: &Stylesheet,
        width: f32,
        scale: f32,
        engine: &mut TextEngine,
    ) -> Result<Self, RenderError> {
        Self::layout_with_assets(
            document,
            style,
            width,
            scale,
            engine,
            &crate::assets::Images::new(),
        )
    }
    pub fn layout_with_assets(
        document: &Document,
        style: &Stylesheet,
        width: f32,
        scale: f32,
        engine: &mut TextEngine,
        images: &crate::assets::Images,
    ) -> Result<Self, RenderError> {
        if !width.is_finite()
            || !(64.0..=16384.0).contains(&width)
            || !scale.is_finite()
            || !(0.5..=4.0).contains(&scale)
        {
            return Err(RenderError(
                "native viewport or scale outside supported bounds",
            ));
        }
        if document.version == ylang::Version::V02 || style.rich.is_some() {
            return rich::layout(document, style, width, scale, engine, images);
        }
        if !(20..=160).contains(&style.width)
            || style.padding > 8
            || [
                style.text.gap,
                style.heading.gap,
                style.section.gap,
                style.link.gap,
            ]
            .iter()
            .any(|gap| *gap > 3)
        {
            return Err(RenderError("stylesheet outside supported layout bounds"));
        }
        let mut metadata_bytes = 0;
        for value in std::iter::once(document.metadata.title.as_str())
            .chain(document.metadata.language.as_deref())
        {
            validate_ast_string(value)?;
            metadata_bytes += value.len();
        }
        let padding = ((style.padding as f32 * 9.0 + 20.0) * scale).min(width / 4.0);
        let content_width = (style.width as f32 * 9.0 * scale).min(width - padding * 2.0);
        let left = (width - content_width) / 2.0;
        let mut builder = DocumentBuilder {
            engine,
            style,
            scale,
            left,
            content_width,
            row: padding,
            total_lines: 0,
            text_bytes: metadata_bytes,
            blocks: Vec::new(),
        };
        builder.nodes(&document.nodes, 0)?;
        Ok(Self {
            width,
            height: builder.row + padding,
            paper: style.paper,
            scale,
            blocks: builder.blocks,
            decorations: Vec::new(),
            images: Vec::new(),
            link_states: None,
        })
    }

    /// Draw only visible shaped blocks, clipping all glyphs to the viewport.
    /// `scroll` and link coordinates are document-relative physical pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        pixmap: &mut Pixmap,
        origin_x: f32,
        origin_y: f32,
        scroll: f32,
        viewport_height: f32,
        engine: &mut TextEngine,
    ) {
        self.draw_interactive(
            pixmap,
            origin_x,
            origin_y,
            scroll,
            viewport_height,
            engine,
            None,
            false,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw_interactive(
        &self,
        pixmap: &mut Pixmap,
        origin_x: f32,
        origin_y: f32,
        scroll: f32,
        viewport_height: f32,
        engine: &mut TextEngine,
        pointer: Option<(f32, f32)>,
        active: bool,
    ) {
        let clip = TextRect {
            x: origin_x,
            y: origin_y,
            width: self.width,
            height: viewport_height,
        };
        if !valid_rect(clip) || !scroll.is_finite() {
            return;
        }
        engine.trim_raster_cache();
        if let Some(rect) = Rect::from_xywh(clip.x, clip.y, clip.width, clip.height) {
            let mut paint = Paint::default();
            let Color(r, g, b) = self.paper;
            paint.set_color_rgba8(r, g, b, 255);
            pixmap.fill_rect(rect, &paint, Transform::identity(), None);
        }
        rich::draw_decorations(&self.decorations, pixmap, origin_x, origin_y, scroll, clip);
        rich::draw_images(&self.images, pixmap, origin_x, origin_y, scroll, clip);
        for block in &self.blocks {
            let y = origin_y + block.y - scroll;
            if y + block.height < origin_y || y > origin_y + viewport_height {
                continue;
            }
            let color = if block.target.is_some()
                && pointer.is_some_and(|(x, y)| block.links.iter().any(|r| r.contains(x, y)))
            {
                self.link_states.map_or(
                    block.color,
                    |(hover, pressed)| if active { pressed } else { hover },
                )
            } else {
                block.color
            };
            paint_buffer(
                &block.buffer,
                &mut engine.fonts,
                &mut engine.swash,
                pixmap,
                origin_x + block.x,
                y,
                color,
                clip,
            );
            if block.target.is_some() {
                let Color(r, g, b) = color;
                let mut paint = Paint::default();
                paint.set_color_rgba8(r, g, b, 150);
                for line in &block.links {
                    let underline_y = origin_y + line.y - scroll + line.height - 2.0 * self.scale;
                    let left = (origin_x + line.x).max(clip.x);
                    let right = (origin_x + line.x + line.width).min(clip.x + clip.width);
                    let top = underline_y.max(clip.y);
                    let bottom = (underline_y + self.scale.max(1.0)).min(clip.y + clip.height);
                    if let Some(rect) = Rect::from_xywh(left, top, right - left, bottom - top) {
                        pixmap.fill_rect(rect, &paint, Transform::identity(), None);
                    }
                }
            }
        }
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.blocks
            .iter()
            .find(|block| {
                block.target.is_some() && block.links.iter().any(|rect| rect.contains(x, y))
            })
            .and_then(|block| block.target.as_deref())
    }
}

fn validate_ast_string(text: &str) -> Result<(), RenderError> {
    if text.len() > MAX_LINE_BYTES {
        return Err(RenderError("AST string exceeds 4096-byte line limit"));
    }
    if text.chars().any(char::is_control) {
        return Err(RenderError("control character in AST"));
    }
    Ok(())
}

struct DocumentBuilder<'a> {
    engine: &'a mut TextEngine,
    style: &'a Stylesheet,
    scale: f32,
    left: f32,
    content_width: f32,
    row: f32,
    total_lines: usize,
    text_bytes: usize,
    blocks: Vec<Block>,
}
impl DocumentBuilder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn block(
        &mut self,
        text: &str,
        depth: usize,
        style: TextStyle,
        size: f32,
        bold: bool,
        target: Option<&str>,
    ) -> Result<(), RenderError> {
        validate_ast_string(text)?;
        if let Some(target) = target {
            validate_ast_string(target)?;
        }
        let bytes = text
            .len()
            .checked_add(target.map_or(0, str::len))
            .ok_or(RenderError("AST text length overflow"))?;
        self.text_bytes = self
            .text_bytes
            .checked_add(bytes)
            .ok_or(RenderError("AST text length overflow"))?;
        if self.text_bytes > MAX_TEXT_BYTES || self.blocks.len() >= MAX_BLOCKS {
            return Err(RenderError("AST exceeds native rendering budget"));
        }
        let indent = (depth as f32 * 18.0 * self.scale)
            .min((self.content_width - 24.0 * self.scale).max(0.0));
        let width = (self.content_width - indent).max(8.0);
        let buffer = self
            .engine
            .shape(text, width, size * self.scale, bold, Wrap::WordOrGlyph);
        let mut lines = 0;
        let mut height: f32 = size * self.scale * 1.5;
        let mut links = Vec::new();
        for line in buffer.layout_runs() {
            lines += 1;
            height = height.max(line.line_top + line.line_height);
            if target.is_some() {
                let (glyph_left, glyph_right) = line.glyphs.iter().fold(
                    (f32::INFINITY, f32::NEG_INFINITY),
                    |(left, right), glyph| (left.min(glyph.x), right.max(glyph.x + glyph.w)),
                );
                let glyph_left = if glyph_left.is_finite() {
                    glyph_left
                } else {
                    0.0
                };
                let glyph_right = if glyph_right.is_finite() {
                    glyph_right
                } else {
                    line.line_w
                };
                links.push(TextRect {
                    x: self.left + indent + glyph_left,
                    y: self.row + line.line_top,
                    width: (glyph_right - glyph_left).max(1.0),
                    height: line.line_height,
                });
            }
        }
        self.total_lines += lines;
        if self.total_lines > MAX_ROWS {
            return Err(RenderError("native layout exceeds 4096 rows"));
        }
        self.blocks.push(Block {
            buffer,
            x: self.left + indent,
            y: self.row,
            height,
            color: style.ink,
            links,
            target: target.map(str::to_owned),
        });
        self.row += height + style.gap as f32 * 12.0 * self.scale;
        Ok(())
    }

    fn nodes(&mut self, nodes: &[Node], depth: usize) -> Result<(), RenderError> {
        if depth > ylang::MAX_DEPTH {
            return Err(RenderError("AST nesting exceeds 32 levels"));
        }
        for node in nodes {
            match &node.kind {
                NodeKind::Heading { level, text } => {
                    let size = match level {
                        1 => 34.0,
                        2 => 29.0,
                        3 => 25.0,
                        _ => 22.0,
                    };
                    self.block(text, depth, self.style.heading, size, true, None)?;
                }
                NodeKind::Section { name, children } => {
                    self.row += 10.0 * self.scale;
                    self.block(name, depth, self.style.section, 21.0, true, None)?;
                    self.nodes(children, depth + 1)?;
                }
                NodeKind::Paragraph(text) => {
                    self.block(text, depth, self.style.text, 17.0, false, None)?
                }
                NodeKind::Link { target, label } => {
                    self.block(label, depth, self.style.link, 17.0, true, Some(target))?
                }
                _ => return Err(RenderError("0.2 AST requires rich native layout")),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Document {
        ylang::parse("@page 0.1\n  title Native document\n= Clear native type\n:: Reading\n  Words wrap using actual shaped glyph advances, including café and naïve.\n  > ynp://127.0.0.1:7302/about | A long clickable link that wraps across narrow views").unwrap()
    }

    #[test]
    fn responsive_layout_and_wrapped_link_hitboxes() {
        let mut engine = TextEngine::new();
        let doc = sample();
        let sheet = Stylesheet::default();
        let narrow = NativeDocument::layout(&doc, &sheet, 280.0, 1.0, &mut engine).unwrap();
        let wide = NativeDocument::layout(&doc, &sheet, 900.0, 1.0, &mut engine).unwrap();
        assert!(narrow.height > wide.height);
        let link = narrow.blocks.last().unwrap();
        assert!(link.links.len() > 1);
        for rect in &link.links {
            assert_eq!(
                narrow.link_at(rect.x + 1.0, rect.y + 1.0),
                Some("ynp://127.0.0.1:7302/about")
            );
            assert_eq!(narrow.link_at(rect.x - 1.0, rect.y + 1.0), None);
        }
    }

    #[test]
    fn glyphs_are_antialiased_and_viewport_clipped() {
        let mut engine = TextEngine::new();
        let sheet = Stylesheet::default();
        let layout = NativeDocument::layout(&sample(), &sheet, 400.0, 1.0, &mut engine).unwrap();
        let mut pixmap = Pixmap::new(440, 220).unwrap();
        pixmap.fill(tiny_skia::Color::WHITE);
        layout.draw(&mut pixmap, 20.0, 20.0, 30.0, 180.0, &mut engine);
        assert_eq!(pixmap.pixel(0, 0).unwrap().red(), 255);
        assert_eq!(pixmap.pixel(439, 219).unwrap().red(), 255);
        assert!(pixmap
            .pixels()
            .iter()
            .any(|p| p.red() > 16 && p.red() < 216 && p.alpha() == 255));
    }

    #[test]
    fn high_dpi_geometry_and_tiny_viewport_remain_bounded() {
        let mut engine = TextEngine::new();
        let sheet = Stylesheet::default();
        let standard = NativeDocument::layout(&sample(), &sheet, 420.0, 1.0, &mut engine).unwrap();
        let retina = NativeDocument::layout(&sample(), &sheet, 840.0, 2.0, &mut engine).unwrap();
        assert!((retina.height - standard.height * 2.0).abs() < 1.0);
        let standard_link = &standard.blocks.last().unwrap().links[0];
        assert_eq!(
            retina.link_at((standard_link.x + 1.0) * 2.0, (standard_link.y + 1.0) * 2.0),
            Some("ynp://127.0.0.1:7302/about")
        );
        let tiny = NativeDocument::layout(&sample(), &sheet, 64.0, 4.0, &mut engine).unwrap();
        let mut pixmap = Pixmap::new(88, 128).unwrap();
        pixmap.fill(tiny_skia::Color::WHITE);
        tiny.draw(&mut pixmap, 12.0, 12.0, 500.0, 90.0, &mut engine);
        assert_eq!(pixmap.pixel(0, 0).unwrap().red(), 255);
        assert_eq!(pixmap.pixel(87, 127).unwrap().red(), 255);
    }

    #[test]
    fn overwide_link_underlines_are_clipped_at_high_dpi() {
        let mut engine = TextEngine::new();
        let doc = ylang::parse("@page 0.1\n  title Clip\n> ynp://127.0.0.1:9/a | WW").unwrap();
        let layout =
            NativeDocument::layout(&doc, &Stylesheet::default(), 64.0, 4.0, &mut engine).unwrap();
        assert!(layout.blocks[0].links[0].x + layout.blocks[0].links[0].width > layout.width);
        let mut image = Pixmap::new(100, 180).unwrap();
        image.fill(tiny_skia::Color::WHITE);
        layout.draw(&mut image, 10.0, 10.0, 0.0, 144.0, &mut engine);
        for y in 0..180 {
            for x in 0..100 {
                if !(10..74).contains(&x) || !(10..154).contains(&y) {
                    assert_eq!(
                        image.pixel(x, y).unwrap(),
                        PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap(),
                        "paint leaked at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn all_ast_strings_count_toward_the_rendering_budget() {
        let mut engine = TextEngine::new();
        let sheet = Stylesheet::default();
        let mut doc = sample();
        doc.metadata.title = "t".repeat(4097);
        assert!(NativeDocument::layout(&doc, &sheet, 400.0, 1.0, &mut engine).is_err());
        doc.metadata.title = "title".into();
        doc.metadata.language = Some("en\u{1b}".into());
        assert!(NativeDocument::layout(&doc, &sheet, 400.0, 1.0, &mut engine).is_err());
        doc.metadata.language = None;
        doc.nodes[0].kind = NodeKind::Link {
            target: "x".repeat(4097),
            label: "short".into(),
        };
        assert!(NativeDocument::layout(&doc, &sheet, 400.0, 1.0, &mut engine).is_err());
        doc.nodes[0].kind = NodeKind::Link {
            target: format!("ynp://{}", "x".repeat(4090)),
            label: "short".into(),
        };
        doc.nodes = vec![doc.nodes[0].clone(); 70];
        let error = NativeDocument::layout(&doc, &sheet, 400.0, 1.0, &mut engine)
            .err()
            .unwrap();
        assert_eq!(error.0, "AST exceeds native rendering budget");
    }

    #[test]
    fn oversized_unicode_ui_prefix_is_bounded_before_cache_lookup() {
        let mut engine = TextEngine::new();
        let oversized = "é".repeat(3000);
        let first = engine.measure(&oversized, 14.0, false);
        assert!(first > 0.0);
        assert_eq!(engine.measure(&oversized, 14.0, false), first);
        assert_eq!(engine.ui_cache.len(), 1);
        assert_eq!(engine.ui_cache[0].text.len(), MAX_LINE_BYTES);
        assert!(engine.ui_cache[0].text.is_char_boundary(MAX_LINE_BYTES));
    }

    #[test]
    fn validates_programmatic_inputs_and_ui_cache_is_bounded() {
        let mut engine = TextEngine::new();
        for width in [0.0, f32::NAN, 20000.0] {
            assert!(NativeDocument::layout(
                &sample(),
                &Stylesheet::default(),
                width,
                1.0,
                &mut engine
            )
            .is_err());
        }
        assert!(
            NativeDocument::layout(&sample(), &Stylesheet::default(), 400.0, 0.0, &mut engine)
                .is_err()
        );
        let mut doc = sample();
        doc.nodes[0].kind = NodeKind::Paragraph("\u{1b}bad".into());
        assert!(
            NativeDocument::layout(&doc, &Stylesheet::default(), 400.0, 1.0, &mut engine).is_err()
        );
        for n in 0..200 {
            assert!(engine.measure(&format!("Tab {n}"), 14.0, false) > 0.0);
        }
        assert_eq!(engine.ui_cache.len(), UI_CACHE_SIZE);
    }
}
