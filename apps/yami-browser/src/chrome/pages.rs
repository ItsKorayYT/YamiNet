//! Reusable native landing-page surfaces, cards and diagnostic layout.
use super::*;

fn surface(c: &mut Canvas<'_>, ui: &ChromeState<'_>) -> Rect {
    let area = viewport(ui.width, ui.height);
    c.round(area, RADIUS, MAIN_BG, MAIN_BG, Some(EDGE));
    c.glow(
        area,
        RADIUS,
        area.x + area.w * 0.7,
        area.y,
        area.w * 0.8,
        [117, 80, 110, 45],
    );
    let width = (area.w - 64.0).min(820.0);
    Rect::new(
        area.x + (area.w - width) / 2.0,
        area.y + (area.h * 0.09).clamp(28.0, 78.0),
        width,
        area.h,
    )
}

fn card(c: &mut Canvas<'_>, r: Rect) {
    c.round(r, 12.0, HOVER_BG, HOVER_BG, Some(EDGE));
}

fn section(c: &mut Canvas<'_>, label: &str, r: Rect) {
    c.text(label, r.x, r.y, r.w, 11.0, ACCENT, true);
}

fn action_card(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    r: Rect,
    icon: &str,
    title: &str,
    caption: &str,
    action: Action,
) {
    let lit = r.contains(ui.mouse.0, ui.mouse.1) || ui.focus == Some(&action);
    c.round(
        r,
        12.0,
        if lit { ACTIVE_TAB } else { HOVER_BG },
        HOVER_BG,
        Some(if lit { ACCENT } else { EDGE }),
    );
    c.icon(icon, r.x + 22.0, r.y + 23.0, ACCENT);
    c.icon("forward", r.x + r.w - 24.0, r.y + 23.0, MUTED);
    c.text(title, r.x + 16.0, r.y + 44.0, r.w - 32.0, 15.0, INK, true);
    c.text(
        caption,
        r.x + 16.0,
        r.y + 71.0,
        r.w - 32.0,
        12.0,
        MUTED,
        false,
    );
    scene.add(r, action);
}

pub fn home(c: &mut Canvas<'_>, scene: &mut Scene, ui: &ChromeState<'_>, browser: &Browser) {
    let body = surface(c, ui);
    let (x, y, w) = (body.x, body.y, body.w);
    section(
        c,
        "YAMINET  /  YOUR STARTING POINT",
        Rect::new(x, y, w, 18.0),
    );
    c.text(
        "A quieter place to explore.",
        x,
        y + 27.0,
        w,
        if w < 650.0 { 30.0 } else { 38.0 },
        INK,
        true,
    );
    c.text(
        "Native documents. Independent nodes. Your next discovery.",
        x,
        y + 87.0,
        w,
        14.0,
        MUTED,
        false,
    );

    address_field(
        c,
        scene,
        Rect::new(x, y + 128.0, w, 50.0),
        Action::Address,
        ui,
        ui.editing,
    );
    c.text(
        "Enter an address · Search saved pages in Discover · Ctrl+L to focus",
        x + 2.0,
        y + 187.0,
        w,
        11.0,
        MUTED,
        false,
    );

    let gap = 12.0;
    let tile_w = (w - 2.0 * gap) / 3.0;
    for (i, (icon, title, caption, action)) in [
        (
            "search",
            "Discover",
            "Explore saved listings",
            Action::Discover,
        ),
        (
            "yami",
            "Welcome",
            "Get to know YamiNet",
            Action::Navigate("local:welcome"),
        ),
        ("plus", "Local games", "Open a game file", Action::OpenGame),
    ]
    .into_iter()
    .enumerate()
    {
        action_card(
            c,
            scene,
            ui,
            Rect::new(x + i as f32 * (tile_w + gap), y + 223.0, tile_w, 105.0),
            icon,
            title,
            caption,
            action,
        );
    }

    let saved = Rect::new(x, y + 350.0, w, 112.0);
    card(c, saved);
    section(
        c,
        "SAVED FOR LATER",
        Rect::new(x + 18.0, saved.y + 14.0, w - 160.0, 18.0),
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w - 120.0, saved.y + 12.0, 102.0, 26.0),
        "Open library",
        Action::Library,
        true,
    );
    let bookmarks = browser
        .library
        .as_ref()
        .map(|l| l.bookmarks())
        .unwrap_or_default();
    if bookmarks.is_empty() {
        c.text(
            "Keep a few favorite places close.",
            x + 18.0,
            saved.y + 47.0,
            w - 36.0,
            14.0,
            INK,
            true,
        );
        c.text(
            if browser.library.is_some() {
                "Save an open page with Ctrl+D. Bookmarks stay on this device."
            } else {
                "Open the library to check local storage and manage bookmarks."
            },
            x + 18.0,
            saved.y + 76.0,
            w - 36.0,
            12.0,
            MUTED,
            false,
        );
    } else {
        let bw = (w - 48.0) / 2.0;
        for (i, bookmark) in bookmarks.iter().take(2).enumerate() {
            panel_button(
                c,
                scene,
                ui,
                Rect::new(x + 18.0 + i as f32 * (bw + 12.0), saved.y + 53.0, bw, 38.0),
                &bookmark.title,
                Action::OpenBookmark(i),
                true,
            );
        }
    }
    c.text(
        "Start here, even offline. Network access is controlled in Settings.",
        x + 2.0,
        saved.y + saved.h + 16.0,
        w,
        11.0,
        MUTED,
        false,
    );
}

/// Measured wrapping preserves every UTF-8 character, including long addresses.
/// Each resulting line fits the same font metrics used by Canvas::text.
fn lines(text: &mut TextEngine, value: &str, width: f32, size: f32, scale: f32) -> Vec<String> {
    let mut result = Vec::new();
    for paragraph in value.split('\n') {
        let mut rest = paragraph;
        while !rest.is_empty() {
            // Bound shaping work per line; long diagnostics must not shape the
            // entire remaining message at every binary-search step.
            let mut bounds: Vec<usize> = rest.char_indices().take(257).map(|(i, _)| i).collect();
            if bounds.len() < 257 {
                bounds.push(rest.len());
            }
            let (mut lo, mut hi) = (1, bounds.len() - 1);
            while lo < hi {
                let mid = (lo + hi).div_ceil(2);
                if text.measure(&rest[..bounds[mid]], size * scale, false) / scale <= width {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            let mut end = bounds[lo];
            if end < rest.len() {
                if let Some((at, ch)) = rest[..end]
                    .char_indices()
                    .rev()
                    .find(|(at, ch)| *at > 0 && ch.is_whitespace())
                {
                    end = at + ch.len_utf8();
                }
            }
            result.push(rest[..end].to_string());
            rest = &rest[end..];
        }
        // Keep explicit paragraph breaks, including empty diagnostics.
        result.push(String::new());
    }
    result
}

fn error_body(width: f32, height: f32) -> Rect {
    let area = viewport(width, height);
    let w = (area.w - 64.0).min(820.0);
    Rect::new(area.x + (area.w - w) / 2.0, area.y + 28.0, w, area.h - 56.0)
}

pub fn error_details_rect(width: f32, height: f32) -> Rect {
    let r = error_body(width, height);
    Rect::new(r.x, r.y + 258.0, r.w, (r.h - 258.0).max(60.0))
}

fn diagnostic_lines(
    text: &mut TextEngine,
    address: &str,
    message: &str,
    r: Rect,
    scale: f32,
) -> Vec<String> {
    lines(
        text,
        &format!("Address: {address}\n{message}"),
        r.w - 52.0,
        13.0,
        scale,
    )
}

pub fn error_scroll_limit(
    text: &mut TextEngine,
    width: f32,
    height: f32,
    scale: f32,
    address: &str,
    message: &str,
) -> f64 {
    let r = error_details_rect(width, height);
    let count = diagnostic_lines(text, address, message, r, scale).len();
    (count as f64 * 21.0 - f64::from(r.h - 56.0)).max(0.0)
}

pub fn error_page(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    browser: &Browser,
    message: &str,
) {
    surface(c, ui);
    let r = error_body(ui.width, ui.height);
    let tab = browser.active();
    let remote = matches!(
        tab.location(),
        crate::model::Location::Yami(_)
            | crate::model::Location::Named(_)
            | crate::model::Location::Ynps(_)
    );
    let badge = Rect::new(r.x, r.y, 144.0, 28.0);
    c.round(badge, 14.0, [64, 40, 42, 255], [64, 40, 42, 255], None);
    c.icon("info", badge.x + 17.0, badge.y + 14.0, PLAINTEXT);
    c.text(
        "PAGE UNAVAILABLE",
        badge.x + 30.0,
        text_y(badge, 10.0),
        108.0,
        10.0,
        PLAINTEXT,
        true,
    );
    c.text("Could not open page", r.x, r.y + 45.0, r.w, 30.0, INK, true);
    c.text(
        "The address may be unavailable or the connection could not be completed.",
        r.x,
        r.y + 100.0,
        r.w,
        13.0,
        MUTED,
        false,
    );
    c.text(
        if remote {
            "Check the full node address and your network settings, then try again."
        } else {
            "Check the address. If this is a local node, make sure it is running."
        },
        r.x,
        r.y + 125.0,
        r.w,
        13.0,
        MUTED,
        false,
    );
    let gap = 10.0;
    let count = if remote { 4.0 } else { 3.0 };
    let bw = ((r.w - (count - 1.0) * gap) / count).min(160.0);
    for (i, (label, action, enabled)) in [
        ("Try again", Action::Reload, true),
        ("Go back", Action::Back, tab.can_back()),
        ("Start page", Action::Navigate(""), true),
        ("Bootstrap peers", Action::Bootstrap, remote),
    ]
    .into_iter()
    .enumerate()
    .take(count as usize)
    {
        panel_button(
            c,
            scene,
            ui,
            Rect::new(r.x + i as f32 * (bw + gap), r.y + 173.0, bw, 38.0),
            label,
            action,
            enabled,
        );
    }
    c.text("Connection details", r.x, r.y + 228.0, r.w, 14.0, INK, true);
    let mut details = error_details_rect(ui.width, ui.height);
    let all = diagnostic_lines(c.text, &tab.address, message, details, c.scale);
    details.h = details.h.min((all.len() as f32 * 21.0 + 56.0).max(140.0));
    card(c, details);
    section(
        c,
        if all.len() as f32 * 21.0 > details.h - 56.0 {
            "DIAGNOSTICS  /  SCROLL TO READ"
        } else {
            "DIAGNOSTICS"
        },
        Rect::new(details.x + 18.0, details.y + 12.0, details.w - 36.0, 18.0),
    );
    let top = details.y + 38.0;
    let bottom = details.y + details.h - 12.0;
    for (index, line) in all.iter().enumerate() {
        let yy = top + index as f32 * 21.0 - tab.scroll as f32;
        if yy >= top && yy + 20.0 <= bottom {
            c.text(
                line,
                details.x + 18.0,
                yy,
                details.w - 52.0,
                13.0,
                MUTED,
                false,
            );
        }
    }
    let limit = (all.len() as f32 * 21.0 - (details.h - 56.0)).max(0.0);
    if limit > 0.0 {
        let track = Rect::new(details.x + details.w - 13.0, top, 5.0, bottom - top);
        let thumb_h = (track.h * track.h / (all.len() as f32 * 21.0)).max(20.0);
        let yy = track.y + (track.h - thumb_h) * (tab.scroll as f32 / limit).clamp(0.0, 1.0);
        c.round(
            Rect::new(track.x, yy, track.w, thumb_h),
            2.5,
            ACCENT,
            ACCENT,
            None,
        );
        scene.add(
            Rect::new(track.x - 5.0, track.y, 15.0, track.h),
            Action::Scrollbar,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_wrap_keeps_unicode_and_long_ids_without_truncation() {
        let mut text = TextEngine::new();
        let value = format!(
            "café 日本語 {} the final diagnostic",
            "yn1-abcdefgh".repeat(130)
        );
        for scale in [1.0, 2.0] {
            let wrapped = lines(&mut text, &value, 310.0, 13.0, scale);
            assert_eq!(wrapped.concat(), value);
            for line in wrapped {
                assert!(text.measure(&line, 13.0 * scale, false) / scale <= 310.1);
            }
        }
    }
}
