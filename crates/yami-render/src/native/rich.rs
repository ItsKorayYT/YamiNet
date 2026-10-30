//! Bounded native 0.2 flow layout, using the original text shaper and painter.
use super::*;
use cosmic_text::{Align as TextAlign, Style};
use tiny_skia::{FillRule, Mask, Path, PathBuilder, Stroke};
use ylang::{ContainerKind, Inline, InlineKind, TextRole};
use ysl::{Align, RichStyle, Typography};
pub(super) struct Decoration {
    rect: TextRect,
    fill: Color,
    border: Option<(Color, f32)>,
    radius: f32,
}
fn rounded(rect: TextRect, radius: f32) -> Option<Path> {
    let (x, y, w, h) = (rect.x, rect.y, rect.width, rect.height);
    if !valid_rect(rect) {
        return None;
    }
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    let mut path = PathBuilder::new();
    path.move_to(x + r, y);
    path.line_to(x + w - r, y);
    path.quad_to(x + w, y, x + w, y + r);
    path.line_to(x + w, y + h - r);
    path.quad_to(x + w, y + h, x + w - r, y + h);
    path.line_to(x + r, y + h);
    path.quad_to(x, y + h, x, y + h - r);
    path.line_to(x, y + r);
    path.quad_to(x, y, x + r, y);
    path.close();
    path.finish()
}
pub(super) fn draw_decorations(
    decorations: &[Decoration],
    pixmap: &mut Pixmap,
    x: f32,
    y: f32,
    scroll: f32,
    clip: TextRect,
) {
    if decorations.is_empty() {
        return;
    }
    let Some(mut mask) = Mask::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    let Some(cliprect) = Rect::from_xywh(clip.x, clip.y, clip.width, clip.height) else {
        return;
    };
    mask.fill_path(
        &PathBuilder::from_rect(cliprect),
        FillRule::Winding,
        false,
        Transform::identity(),
    );
    for decoration in decorations {
        let rect = TextRect {
            x: x + decoration.rect.x,
            y: y + decoration.rect.y - scroll,
            ..decoration.rect
        };
        if rect.y + rect.height < clip.y || rect.y > clip.y + clip.height {
            continue;
        }
        if let Some(path) = rounded(rect, decoration.radius) {
            let mut paint = Paint::default();
            let Color(r, g, b) = decoration.fill;
            paint.set_color_rgba8(r, g, b, 255);
            pixmap.fill_path(
                &path,
                &paint,
                FillRule::Winding,
                Transform::identity(),
                Some(&mask),
            );
            if let Some((Color(r, g, b), width)) = decoration.border {
                paint.set_color_rgba8(r, g, b, 255);
                pixmap.stroke_path(
                    &path,
                    &paint,
                    &Stroke {
                        width,
                        ..Stroke::default()
                    },
                    Transform::identity(),
                    Some(&mask),
                );
            }
        }
    }
}
fn plain(text: &str) -> Vec<Inline> {
    vec![Inline {
        kind: InlineKind::Plain,
        text: text.into(),
    }]
}
pub(super) fn layout(
    document: &Document,
    sheet: &Stylesheet,
    width: f32,
    scale: f32,
    engine: &mut TextEngine,
) -> Result<NativeDocument, RenderError> {
    let style = sheet.rich.clone().unwrap_or_else(|| RichStyle {
        width: (sheet.width * 9).clamp(240, 1440) as u16,
        ..RichStyle::default()
    });
    if !style.valid() {
        return Err(RenderError("0.2 stylesheet outside supported bounds"));
    }
    let mut bytes =
        document.metadata.title.len() + document.metadata.language.as_ref().map_or(0, String::len);
    validate_ast_string(&document.metadata.title)?;
    if let Some(language) = &document.metadata.language {
        validate_ast_string(language)?;
    }
    if bytes > MAX_TEXT_BYTES {
        return Err(RenderError("metadata exceeds rendering budget"));
    }
    let padding = (style.padding as f32 * scale).min(width / 4.0);
    let content = (style.width as f32 * scale)
        .min(width - padding * 2.0)
        .max(16.0);
    let left = (width - content) / 2.0;
    let mut builder = Builder {
        engine,
        sheet,
        style: &style,
        scale,
        blocks: Vec::new(),
        decorations: Vec::new(),
        lines: 0,
        nodes: 0,
        bytes: &mut bytes,
    };
    let bottom = builder.nodes(&document.nodes, left, padding, content, 0, None)?;
    Ok(NativeDocument {
        width,
        height: bottom + padding,
        paper: sheet.paper,
        scale,
        blocks: builder.blocks,
        decorations: builder.decorations,
        link_states: Some((style.hover, style.active)),
    })
}
struct Builder<'a> {
    engine: &'a mut TextEngine,
    sheet: &'a Stylesheet,
    style: &'a RichStyle,
    scale: f32,
    blocks: Vec<Block>,
    decorations: Vec<Decoration>,
    lines: usize,
    nodes: usize,
    bytes: &'a mut usize,
}
impl Builder<'_> {
    fn account(&mut self, text: &str) -> Result<(), RenderError> {
        if text.len() > 16 * 1024 || text.chars().any(char::is_control) {
            return Err(RenderError("invalid or oversized 0.2 AST text"));
        }
        *self.bytes = self
            .bytes
            .checked_add(text.len())
            .ok_or(RenderError("text budget overflow"))?;
        if *self.bytes > MAX_TEXT_BYTES {
            return Err(RenderError("0.2 AST exceeds text budget"));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        content: &[Inline],
        x: f32,
        y: f32,
        width: f32,
        typography: Typography,
        color: Color,
        target: Option<&str>,
    ) -> Result<f32, RenderError> {
        for part in content {
            self.account(&part.text)?;
        }
        if let Some(target) = target {
            self.account(target)?;
        }
        if self.blocks.len() >= MAX_BLOCKS {
            return Err(RenderError("native block budget exceeded"));
        }
        let size = typography.size as f32 * self.scale;
        let line_height = size * typography.line_height as f32 / 100.0;
        let mut buffer = Buffer::new(&mut self.engine.fonts, Metrics::new(size, line_height));
        buffer.set_size(&mut self.engine.fonts, Some(width.max(8.0)), None);
        buffer.set_wrap(&mut self.engine.fonts, Wrap::WordOrGlyph);
        let base = Attrs::new()
            .family(Family::Name("Open Sans"))
            .weight(Weight(typography.weight));
        let spans = content.iter().map(|part| {
            let attrs = match part.kind {
                InlineKind::Plain => base.clone(),
                InlineKind::Strong => base.clone().weight(Weight(600)),
                InlineKind::Emphasis => base
                    .clone()
                    .family(Family::Name("DejaVu Sans"))
                    .style(Style::Italic),
                InlineKind::Code => base
                    .clone()
                    .family(Family::Name("DejaVu Sans Mono"))
                    .weight(Weight(400)),
            };
            (part.text.as_str(), attrs)
        });
        let alignment = match typography.align {
            Align::Left => TextAlign::Left,
            Align::Center => TextAlign::Center,
            Align::Right => TextAlign::Right,
        };
        buffer.set_rich_text(
            &mut self.engine.fonts,
            spans,
            &base,
            Shaping::Advanced,
            Some(alignment),
        );
        buffer.shape_until_scroll(&mut self.engine.fonts, false);
        let mut height = line_height;
        let mut links = Vec::new();
        for run in buffer.layout_runs() {
            self.lines += 1;
            height = height.max(run.line_top + run.line_height);
            if self.lines > MAX_ROWS {
                return Err(RenderError("native layout exceeds 4096 rows"));
            }
            if target.is_some() {
                let (left, right) = run
                    .glyphs
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), glyph| {
                        (a.min(glyph.x), b.max(glyph.x + glyph.w))
                    });
                if left.is_finite() && right.is_finite() {
                    links.push(TextRect {
                        x: x + left,
                        y: y + run.line_top,
                        width: (right - left).max(1.0),
                        height: run.line_height,
                    });
                }
            }
        }
        self.blocks.push(Block {
            buffer,
            x,
            y,
            height,
            color,
            links,
            target: target.map(str::to_owned),
        });
        Ok(y + height + typography.gap as f32 * self.scale)
    }
    fn nodes(
        &mut self,
        nodes: &[Node],
        x: f32,
        mut y: f32,
        width: f32,
        depth: usize,
        ink: Option<Color>,
    ) -> Result<f32, RenderError> {
        if depth > ylang::MAX_DEPTH {
            return Err(RenderError("AST nesting exceeds 32 levels"));
        }
        for node in nodes {
            self.nodes += 1;
            if self.nodes > ylang::MAX_NODES || y > 131072.0 * self.scale {
                return Err(RenderError("rich document exceeds layout budget"));
            }
            match &node.kind {
                NodeKind::Text { role, content } => {
                    let (mut font, color) = match role {
                        TextRole::Paragraph => {
                            (self.style.text, ink.unwrap_or(self.sheet.text.ink))
                        }
                        TextRole::Heading(_) => (self.style.heading, self.sheet.heading.ink),
                    };
                    if let TextRole::Heading(level) = role {
                        if !(1..=6).contains(level) {
                            return Err(RenderError("invalid heading level"));
                        }
                        font.size = (font.size as f32 * (1.0 - (*level as f32 - 1.0) * 0.09))
                            .max(12.0) as u16;
                    }
                    y = self.text(content, x, y, width, font, color, None)?;
                }
                NodeKind::Heading { level, text } => {
                    let content = plain(text);
                    let mut font = self.style.heading;
                    font.size =
                        (font.size as f32 * (1.0 - (*level as f32 - 1.0) * 0.09)).max(12.0) as u16;
                    y = self.text(&content, x, y, width, font, self.sheet.heading.ink, None)?;
                }
                NodeKind::Paragraph(text) => {
                    y = self.text(
                        &plain(text),
                        x,
                        y,
                        width,
                        self.style.text,
                        ink.unwrap_or(self.sheet.text.ink),
                        None,
                    )?
                }
                NodeKind::Link { target, label } => {
                    y = self.text(
                        &plain(label),
                        x,
                        y,
                        width,
                        self.style.link,
                        self.sheet.link.ink,
                        Some(target),
                    )?
                }
                NodeKind::Section { name, children } => {
                    y = self.text(
                        &plain(name),
                        x,
                        y,
                        width,
                        self.style.section,
                        self.sheet.section.ink,
                        None,
                    )?;
                    let indent = (16.0 * self.scale).min(width / 4.0);
                    y = self.nodes(children, x + indent, y, width - indent, depth + 1, ink)?;
                }
                NodeKind::Container {
                    kind,
                    name,
                    children,
                } => {
                    self.account(name)?;
                    match kind {
                        ContainerKind::Card => {
                            let card = self.style.card;
                            let padding = (card.padding as f32 * self.scale).min(width / 4.0);
                            let start = y;
                            let index = self.decorations.len();
                            self.decorations.push(Decoration {
                                rect: TextRect {
                                    x,
                                    y,
                                    width,
                                    height: 1.0,
                                },
                                fill: card.paper,
                                border: (card.border_width > 0).then_some((
                                    card.border,
                                    card.border_width as f32 * self.scale,
                                )),
                                radius: card.radius as f32 * self.scale,
                            });
                            y += padding;
                            if !name.is_empty() {
                                y = self.text(
                                    &plain(name),
                                    x + padding,
                                    y,
                                    width - 2.0 * padding,
                                    self.style.section,
                                    self.sheet.section.ink,
                                    None,
                                )?;
                            }
                            y = self.nodes(
                                children,
                                x + padding,
                                y,
                                (width - 2.0 * padding).max(8.0),
                                depth + 1,
                                Some(card.ink),
                            )? + padding;
                            self.decorations[index].rect.height = (y - start).max(1.0);
                            y += card.gap as f32 * self.scale;
                        }
                        ContainerKind::Row => {
                            if children.len() > 4 {
                                return Err(RenderError("row exceeds four children"));
                            }
                            if !name.is_empty() {
                                y = self.text(
                                    &plain(name),
                                    x,
                                    y,
                                    width,
                                    self.style.section,
                                    self.sheet.section.ink,
                                    None,
                                )?;
                            }
                            let gap = self.style.row_gap as f32 * self.scale;
                            let columns = children.len();
                            if columns > 1
                                && width
                                    >= columns as f32 * self.style.row_min_width as f32 * self.scale
                                        + gap * (columns - 1) as f32
                            {
                                let column = (width - gap * (columns - 1) as f32) / columns as f32;
                                let mut bottom = y;
                                for (index, child) in children.iter().enumerate() {
                                    bottom = bottom.max(self.nodes(
                                        std::slice::from_ref(child),
                                        x + index as f32 * (column + gap),
                                        y,
                                        column,
                                        depth + 1,
                                        ink,
                                    )?);
                                }
                                y = bottom + gap;
                            } else {
                                y = self.nodes(children, x, y, width, depth + 1, ink)?;
                            }
                        }
                        ContainerKind::Group => {
                            if !name.is_empty() {
                                y = self.text(
                                    &plain(name),
                                    x,
                                    y,
                                    width,
                                    self.style.section,
                                    self.sheet.section.ink,
                                    None,
                                )?;
                            }
                            y = self.nodes(children, x, y, width, depth + 1, ink)?;
                        }
                    }
                }
                NodeKind::List { ordered, items } => {
                    for (index, item) in items.iter().enumerate() {
                        let NodeKind::ListItem { content, children } = &item.kind else {
                            return Err(RenderError("invalid list item AST"));
                        };
                        self.nodes += 1;
                        if self.nodes > ylang::MAX_NODES {
                            return Err(RenderError("node budget exceeded"));
                        }
                        let mut runs = vec![Inline {
                            kind: InlineKind::Plain,
                            text: if *ordered {
                                format!("{}. ", index + 1)
                            } else {
                                "• ".into()
                            },
                        }];
                        runs.extend(content.iter().cloned());
                        y = self.text(
                            &runs,
                            x,
                            y,
                            width,
                            self.style.text,
                            ink.unwrap_or(self.sheet.text.ink),
                            None,
                        )?;
                        let indent = (20.0 * self.scale).min(width / 4.0);
                        y = self.nodes(children, x + indent, y, width - indent, depth + 1, ink)?;
                    }
                }
                NodeKind::ListItem { .. } => return Err(RenderError("orphan list item")),
                NodeKind::Image { alt, key } => {
                    self.account(key)?;
                    y = self.text(
                        &plain(&format!("[Image: {alt}]")),
                        x,
                        y,
                        width,
                        self.style.text,
                        ink.unwrap_or(self.sheet.text.ink),
                        None,
                    )?;
                }
                NodeKind::Separator => {
                    let height = self.scale;
                    self.decorations.push(Decoration {
                        rect: TextRect {
                            x,
                            y: y + 8.0 * self.scale,
                            width,
                            height,
                        },
                        fill: self.style.card.border,
                        border: None,
                        radius: 0.0,
                    });
                    y += 24.0 * self.scale;
                }
            }
        }
        Ok(y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rich_cards_rows_text_and_visual_link_hits_wrap_responsively() {
        let document=ylang::parse("@page 0.2\n  title Styled\n= A [strong: styled] page\n:: row\n  :: card First\n    Text with [em: emphasis] and [code: monospace].\n    > /about | A link long enough to wrap in a compact card\n  :: card Second\n    Another block of readable text with Unicode café.\n---\n").unwrap();
        let sheet = ysl::parse("@style 0.2\ncanvas\n  width 960\ntext\n  align center\n").unwrap();
        let mut engine = TextEngine::new();
        let narrow = NativeDocument::layout(&document, &sheet, 280.0, 1.0, &mut engine).unwrap();
        let wide = NativeDocument::layout(&document, &sheet, 1000.0, 1.0, &mut engine).unwrap();
        assert!(narrow.height > wide.height);
        assert_eq!(wide.decorations.len(), 3);
        let link = narrow.blocks.iter().find(|b| b.target.is_some()).unwrap();
        for rect in &link.links {
            assert_eq!(narrow.link_at(rect.x + 0.5, rect.y + 0.5), Some("/about"));
        }
        let rect = link.links[0];
        let mut normal = Pixmap::new(300, 900).unwrap();
        normal.fill(tiny_skia::Color::WHITE);
        narrow.draw(&mut normal, 10.0, 10.0, 0.0, 880.0, &mut engine);
        let mut hovered = Pixmap::new(300, 900).unwrap();
        hovered.fill(tiny_skia::Color::WHITE);
        narrow.draw_interactive(
            &mut hovered,
            10.0,
            10.0,
            0.0,
            880.0,
            &mut engine,
            Some((rect.x + 0.5, rect.y + 0.5)),
            false,
        );
        assert_ne!(normal.data(), hovered.data());
        assert_eq!(hovered.pixel(0, 0), normal.pixel(0, 0));
        assert_eq!(hovered.pixel(0, 0).unwrap().red(), 255);
    }
    #[test]
    fn rich_programmatic_bounds_fail_without_unbounded_layout() {
        let mut doc = ylang::parse("@page 0.2\n  title Bounds\nHello\n").unwrap();
        let mut sheet = ysl::parse("@style 0.2").unwrap();
        let mut engine = TextEngine::new();
        sheet.rich.as_mut().unwrap().text.size = u16::MAX;
        assert!(NativeDocument::layout(&doc, &sheet, 800.0, 1.0, &mut engine).is_err());
        sheet.rich = Some(RichStyle::default());
        doc.nodes[0].kind = NodeKind::Text {
            role: TextRole::Paragraph,
            content: plain(&"x".repeat(16 * 1024 + 1)),
        };
        assert!(NativeDocument::layout(&doc, &sheet, 800.0, 1.0, &mut engine).is_err());
    }
}
