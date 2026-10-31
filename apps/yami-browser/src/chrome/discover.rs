//! Native, modal directory UI, painted with the same browser canvas and editor.
use super::*;
use yami_browser::discover::{Discover, Field};
use yami_discover::SourceState;

fn cached_label(source: &yami_search::Subscription) -> String {
    let age = source.last_success.and_then(|at| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|now| now.as_secs().saturating_sub(at))
    });
    format!(
        "{}{}",
        source.status().label(),
        age.map(|s| format!(" · refreshed {s}s ago"))
            .unwrap_or_default()
    )
}

fn pages_body(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    panel: &Discover,
    editing: Option<(Field, &str)>,
    local_only: bool,
    r: Rect,
) {
    let pages = &panel.pages;
    let x = r.x + 16.0;
    let w = r.w - 32.0;
    let input = |c: &mut Canvas<'_>, scene: &mut Scene, field: Field, value: &str, rect: Rect| {
        let focused = editing.is_some_and(|(active, _)| active == field);
        let field_ui = ChromeState {
            value: editing
                .filter(|(active, _)| *active == field)
                .map_or(value, |(_, v)| v),
            editing: focused,
            selection: ui.selection,
            selection_range: ui.selection_range.clone(),
            caret: ui.caret,
            ..*ui
        };
        address_field(
            c,
            scene,
            rect,
            Action::DiscoverField(field),
            &field_ui,
            focused,
        );
    };
    let message = pages
        .busy
        .or(pages.error.as_deref())
        .unwrap_or(if panel.directories {
            "Adding never fetches. Select a provider to refresh explicitly."
        } else {
            "All saved providers · queries stay on this device · cached results work offline."
        });
    let (count, top, stride) = if panel.directories {
        c.text("Provider name", x, r.y + 109.0, w, 11.0, MUTED, false);
        input(
            c,
            scene,
            Field::Name,
            &pages.name,
            Rect::new(x, r.y + 126.0, w, 28.0),
        );
        c.text(
            "Provider manifest address (yami://)",
            x,
            r.y + 164.0,
            w,
            11.0,
            MUTED,
            false,
        );
        input(
            c,
            scene,
            Field::Address,
            &pages.address,
            Rect::new(x, r.y + 181.0, w, 28.0),
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + 217.0, w, 26.0),
            "Add search provider (no fetch)",
            Action::DiscoverAdd,
            pages.busy.is_none(),
        );
        lines(
            c,
            message,
            x,
            r.y + 250.0,
            w,
            2,
            if pages.error.is_some() {
                PLAINTEXT
            } else {
                MUTED
            },
        );
        (pages.sources.len(), r.y + 286.0, 104.0)
    } else {
        input(
            c,
            scene,
            Field::Search,
            &pages.query,
            Rect::new(x, r.y + 119.0, w - 86.0, 30.0),
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x + w - 78.0, r.y + 119.0, 78.0, 30.0),
            "Search",
            Action::DiscoverSearch,
            true,
        );
        lines(
            c,
            message,
            x,
            r.y + 157.0,
            w,
            2,
            if pages.error.is_some() {
                PLAINTEXT
            } else {
                MUTED
            },
        );
        (pages.hits.len(), r.y + 197.0, 166.0)
    };
    let size = discover_page_size(ui.height, panel.directories);
    let page = pages.page.min(count.saturating_sub(1) / size);
    if count == 0 && pages.busy.is_none() {
        let message = if pages.sources.is_empty() {
            "No search providers. Open Search providers to add an independent provider."
        } else if panel.directories {
            "No saved providers."
        } else if pages.query.is_empty() {
            "Enter a page query. Refresh a selected provider explicitly to download its index."
        } else {
            "No matching cached pages. Try another query or explicitly refresh a provider."
        };
        lines(c, message, x, top, w, 3, MUTED);
    }
    if panel.directories {
        for (index, source) in pages
            .sources
            .iter()
            .enumerate()
            .skip(page * size)
            .take(size)
        {
            let y = top + (index - page * size) as f32 * stride;
            let selected = pages.selected_source.as_deref() == Some(&source.name);
            let card = Rect::new(x, y, w, 98.0);
            c.round(
                card,
                6.0,
                if selected { ACTIVE_TAB } else { HOVER_BG },
                HOVER_BG,
                selected.then_some(PLUM_GLOW),
            );
            scene.add(card, Action::DiscoverSource(source.name.clone()));
            c.text(
                &format!("{} · {}", source.name, cached_label(source)),
                x + 8.0,
                y + 4.0,
                w - 16.0,
                12.0,
                INK,
                true,
            );
            lines(c, &source.address, x + 8.0, y + 25.0, w - 16.0, 2, MUTED);
            lines(
                c,
                source.last_error.as_deref().unwrap_or(
                    "Publisher identity verifies source only; destination claims remain untrusted.",
                ),
                x + 8.0,
                y + 60.0,
                w - 16.0,
                2,
                MUTED,
            );
        }
        let selected = pages.selected_source.is_some() && pages.busy.is_none();
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + r.h - 65.0, w * 0.62, 26.0),
            if pages.busy.is_some() {
                "Cancel operation"
            } else if local_only {
                "Refresh blocked: local-only"
            } else {
                "Refresh selected provider"
            },
            if pages.busy.is_some() {
                Action::DiscoverCancel
            } else {
                Action::DiscoverRefresh
            },
            pages.busy.is_some() || (selected && !local_only),
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x + w * 0.65, r.y + r.h - 65.0, w * 0.35, 26.0),
            "Remove selected",
            Action::DiscoverRemove,
            selected,
        );
    } else {
        for (index, result) in pages.hits.iter().enumerate().skip(page * size).take(size) {
            let y = top + (index - page * size) as f32 * stride;
            let hit = &result.hit;
            let address = hit.page.address.as_deref().unwrap_or("");
            let selected = pages.selected_page.as_deref() == Some(address);
            let card = Rect::new(x, y, w, 158.0);
            c.round(
                card,
                6.0,
                if selected { ACTIVE_TAB } else { HOVER_BG },
                HOVER_BG,
                selected.then_some(PLUM_GLOW),
            );
            scene.add(card, Action::DiscoverWebsite(address.into()));
            c.text(
                &format!("{} · relevance {}", hit.page.title, hit.score),
                x + 10.0,
                y + 6.0,
                w - 20.0,
                13.0,
                INK,
                true,
            );
            lines(c, &hit.snippet, x + 10.0, y + 28.0, w - 20.0, 2, INK);
            lines(
                c,
                &format!("Provider: {}", hit.source),
                x + 10.0,
                y + 64.0,
                w - 20.0,
                2,
                MUTED,
            );
            let source = pages.sources.iter().find(|s| s.address == hit.source);
            let freshness = source
                .map(cached_label)
                .unwrap_or_else(|| result.status.label().into());
            c.text(
                &format!(
                    "{freshness} · {} source(s) · untrusted claim",
                    result.providers.len()
                ),
                x + 10.0,
                y + 98.0,
                w - 20.0,
                11.0,
                MUTED,
                false,
            );
            lines(c, address, x + 10.0, y + 118.0, w - 20.0, 2, MUTED);
        }
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + r.h - 65.0, w, 26.0),
            "Open selected page · verify destination identity",
            Action::DiscoverOpen,
            pages.selected_page.is_some(),
        );
    }
    let bottom = r.y + r.h - 31.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, bottom, 78.0, 23.0),
        "Previous",
        Action::DiscoverPage(false),
        page > 0,
    );
    c.text(
        &format!(
            "{count} results · page {} / {}",
            page + 1,
            count.div_ceil(size).max(1)
        ),
        x + 90.0,
        bottom + 3.0,
        w - 178.0,
        11.0,
        MUTED,
        false,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w - 70.0, bottom, 70.0, 23.0),
        "Next",
        Action::DiscoverPage(true),
        (page + 1) * size < count,
    );
}

pub fn discover_rect(width: f32, height: f32) -> Rect {
    Rect::new(
        SIDEBAR_WIDTH + 16.0,
        20.0,
        (width - SIDEBAR_WIDTH - 40.0).clamp(1.0, 1000.0),
        (height - 40.0).max(1.0),
    )
}
pub fn discover_page_size(height: f32, directories: bool) -> usize {
    let available = height - 40.0 - if directories { 356.0 } else { 270.0 };
    (available / if directories { 104.0 } else { 166.0 })
        .floor()
        .max(1.0) as usize
}
fn lines(c: &mut Canvas<'_>, text: &str, x: f32, y: f32, width: f32, max: usize, ink: [u8; 4]) {
    let mut rest = text.trim();
    for line in 0..max {
        if rest.is_empty() {
            break;
        }
        if c.text.measure(rest, 11.0 * c.scale, false) / c.scale <= width {
            c.text(rest, x, y + line as f32 * 16.0, width, 11.0, ink, false);
            break;
        }
        let boundaries: Vec<_> = rest
            .char_indices()
            .map(|(i, _)| i)
            .chain(std::iter::once(rest.len()))
            .collect();
        let (mut low, mut high) = (1, boundaries.len() - 1);
        while low < high {
            let mid = (low + high).div_ceil(2);
            if c.text
                .measure(&rest[..boundaries[mid]], 11.0 * c.scale, false)
                / c.scale
                <= width - 14.0
            {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        let mut end = boundaries[low];
        if end < rest.len() {
            if let Some(space) = rest[..end].rfind(' ').filter(|&i| i > end / 2) {
                end = space;
            }
        }
        let label = if line + 1 == max && end < rest.len() {
            format!("{}…", &rest[..end])
        } else {
            rest[..end].into()
        };
        c.text(&label, x, y + line as f32 * 16.0, width, 11.0, ink, false);
        rest = rest[end..].trim_start();
    }
}
pub fn discover_panel(
    c: &mut Canvas<'_>,
    scene: &mut Scene,
    ui: &ChromeState<'_>,
    panel: &Discover,
    editing: Option<(Field, &str)>,
    local_only: bool,
) {
    if !panel.open {
        return;
    }
    let r = discover_rect(ui.width, ui.height);
    // Underlying document controls must not be reachable by mouse or Tab.
    scene.hits.retain(|hit| hit.rect.x < SIDEBAR_WIDTH);
    c.round(r, 10.0, SIDEBAR_BG, SIDEBAR_BG, Some(MUTED));
    scene.add(r, Action::DiscoveryPanel);
    let x = r.x + 16.0;
    let w = r.w - 32.0;
    c.text("Discover", x, r.y + 12.0, w - 76.0, 19.0, INK, true);
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w - 65.0, r.y + 10.0, 65.0, 26.0),
        "Close",
        Action::Discover,
        true,
    );
    lines(
        c,
        "Untrusted listings. Opening a website independently verifies its node over TLS.",
        x,
        r.y + 42.0,
        w,
        2,
        MUTED,
    );
    let tabs_y = r.y + 78.0;
    for (index, (label, action)) in [
        ("Websites", Action::DiscoverMode(false)),
        ("Pages", Action::DiscoverMode(true)),
        (
            if panel.pages_mode {
                "Search providers"
            } else {
                "Directories"
            },
            Action::DiscoverView(true),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x + w * 0.34 * index as f32, tabs_y, w * 0.32, 27.0),
            label,
            action,
            true,
        );
    }
    let active = if panel.directories {
        2
    } else {
        usize::from(panel.pages_mode)
    };
    let active_x = x + w * 0.34 * active as f32;
    c.line(
        &[
            (active_x + 4.0, tabs_y + 27.0),
            (active_x + w * 0.32 - 4.0, tabs_y + 27.0),
        ],
        PLUM_GLOW,
        2.0,
    );
    if panel.pages_mode {
        pages_body(c, scene, ui, panel, editing, local_only, r);
        return;
    }
    let input = |c: &mut Canvas<'_>, scene: &mut Scene, field: Field, value: &str, rect: Rect| {
        let focused = editing.is_some_and(|(active, _)| active == field);
        let field_ui = ChromeState {
            value: editing
                .filter(|(active, _)| *active == field)
                .map_or(value, |(_, value)| value),
            editing: focused,
            selection: ui.selection,
            selection_range: ui.selection_range.clone(),
            caret: ui.caret,
            ..*ui
        };
        address_field(
            c,
            scene,
            rect,
            Action::DiscoverField(field),
            &field_ui,
            focused,
        );
    };
    let message = panel
        .busy
        .or(panel.query_error.as_deref())
        .or(panel.error.as_deref())
        .unwrap_or(if panel.directories {
            "Adding a subscription never fetches it. Select one to refresh explicitly."
        } else {
            "Search uses saved listings only. Fresh means successful at the last refresh."
        });
    let count;
    let top;
    let stride;
    if panel.directories {
        c.text("Label", x, r.y + 113.0, w, 11.0, MUTED, false);
        input(
            c,
            scene,
            Field::Name,
            &panel.name,
            Rect::new(x, r.y + 130.0, w, 28.0),
        );
        c.text("Directory address", x, r.y + 164.0, w, 11.0, MUTED, false);
        input(
            c,
            scene,
            Field::Address,
            &panel.address,
            Rect::new(x, r.y + 181.0, w, 28.0),
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + 217.0, w, 26.0),
            "Add subscription (no fetch)",
            Action::DiscoverAdd,
            panel.busy.is_none(),
        );
        lines(
            c,
            message,
            x,
            r.y + 250.0,
            w,
            2,
            if panel.error.is_some() {
                PLAINTEXT
            } else {
                MUTED
            },
        );
        count = panel.sources.len();
        top = r.y + 286.0;
        stride = 104.0;
    } else {
        input(
            c,
            scene,
            Field::Search,
            &panel.query,
            Rect::new(x, r.y + 119.0, w - 86.0, 30.0),
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x + w - 78.0, r.y + 119.0, 78.0, 30.0),
            "Search",
            Action::DiscoverSearch,
            true,
        );
        lines(
            c,
            message,
            x,
            r.y + 157.0,
            w,
            2,
            if panel.error.is_some() {
                PLAINTEXT
            } else {
                MUTED
            },
        );
        count = panel.listings.len();
        top = r.y + 197.0;
        stride = 166.0;
    }
    let size = discover_page_size(ui.height, panel.directories);
    let page = panel.page.min(count.saturating_sub(1) / size);
    if count == 0 && panel.busy.is_none() {
        let text = if panel.directories {
            "No directories saved. Add a canonical yami:// subscription above."
        } else if panel.sources.is_empty() {
            "No saved listings. Add a directory subscription, then explicitly refresh it."
        } else if !panel.query.is_empty() {
            "No matching saved websites. Try another local query."
        } else {
            "No cached listings yet. Select a directory and refresh it explicitly."
        };
        lines(c, text, x, top, w, 3, MUTED);
    }
    if panel.directories {
        for (index, source) in panel
            .sources
            .iter()
            .enumerate()
            .skip(page * size)
            .take(size)
        {
            let y = top + (index - page * size) as f32 * stride;
            let selected = panel.selected_source.as_deref() == Some(source.name());
            c.round(
                Rect::new(x, y, w, 98.0),
                6.0,
                if selected { ACTIVE_TAB } else { HOVER_BG },
                HOVER_BG,
                selected.then_some(PLUM_GLOW),
            );
            scene.add(
                Rect::new(x, y, w, 98.0),
                Action::DiscoverSource(source.name().into()),
            );
            let state = panel.source_state(source.name());
            c.text(
                &format!(
                    "{} · {}",
                    source.name(),
                    state.map_or("not built", SourceState::label)
                ),
                x + 8.0,
                y + 4.0,
                w - 16.0,
                12.0,
                INK,
                true,
            );
            let address = source
                .remote_address()
                .map(|a| a.url())
                .unwrap_or_else(|| source.local_path().unwrap().display().to_string());
            lines(c, &address, x + 8.0, y + 25.0, w - 16.0, 2, MUTED);
            lines(
                c,
                state
                    .and_then(SourceState::reason)
                    .unwrap_or("Untrusted source · refresh is explicit"),
                x + 8.0,
                y + 60.0,
                w - 16.0,
                2,
                MUTED,
            );
        }
        let selected = panel.selected_source.is_some() && panel.busy.is_none();
        let blocked = local_only
            && panel.sources.iter().any(|source| {
                Some(source.name()) == panel.selected_source.as_deref()
                    && source.remote_address().is_some()
            });
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + r.h - 65.0, w * 0.62, 26.0),
            if blocked {
                "Refresh blocked: local-only"
            } else {
                "Refresh selected directory"
            },
            Action::DiscoverRefresh,
            selected && !blocked,
        );
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x + w * 0.65, r.y + r.h - 65.0, w * 0.35, 26.0),
            "Remove selected",
            Action::DiscoverRemove,
            selected,
        );
    } else {
        for (index, listing) in panel
            .listings
            .iter()
            .enumerate()
            .skip(page * size)
            .take(size)
        {
            let y = top + (index - page * size) as f32 * stride;
            let entry = &listing.entry;
            let selected =
                panel.selected_website.as_deref() == Some(entry.address().url().as_str());
            let card = Rect::new(x, y, w, 158.0);
            c.round(
                card,
                6.0,
                if selected { ACTIVE_TAB } else { HOVER_BG },
                HOVER_BG,
                selected.then_some(PLUM_GLOW),
            );
            scene.add(card, Action::DiscoverWebsite(entry.address().url()));
            c.text(entry.title(), x + 10.0, y + 6.0, w - 20.0, 13.0, INK, true);
            lines(c, entry.description(), x + 10.0, y + 30.0, w - 20.0, 3, INK);
            c.text(
                &format!("Category: {}", entry.category().unwrap_or("Uncategorized")),
                x + 10.0,
                y + 80.0,
                w - 20.0,
                11.0,
                MUTED,
                false,
            );
            c.text(
                &format!(
                    "Directory: {} · {} · untrusted",
                    entry.source(),
                    listing.state.label()
                ),
                x + 10.0,
                y + 98.0,
                w - 20.0,
                11.0,
                MUTED,
                false,
            );
            lines(
                c,
                &entry.address().url(),
                x + 10.0,
                y + 118.0,
                w - 20.0,
                2,
                MUTED,
            );
        }
        panel_button(
            c,
            scene,
            ui,
            Rect::new(x, r.y + r.h - 65.0, w, 26.0),
            "Open selected website · verify identity",
            Action::DiscoverOpen,
            panel.selected_website.is_some(),
        );
    }
    let bottom = r.y + r.h - 31.0;
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x, bottom, 78.0, 23.0),
        "Previous",
        Action::DiscoverPage(false),
        page > 0,
    );
    c.text(
        &format!(
            "{} results · page {} / {}",
            count,
            page + 1,
            count.div_ceil(size).max(1)
        ),
        x + 90.0,
        bottom + 3.0,
        w - 178.0,
        11.0,
        MUTED,
        false,
    );
    panel_button(
        c,
        scene,
        ui,
        Rect::new(x + w - 70.0, bottom, 70.0, 23.0),
        "Next",
        Action::DiscoverPage(true),
        (page + 1) * size < count,
    );
}
