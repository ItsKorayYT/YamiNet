//! Native window/event adapter. All network and parsing work lives in Loader.
use crate::{
    chrome::{self, Action, Canvas, ChromeState, Rect, Scene, INK},
    model::{Browser, Loader, Location, PageState},
};
use softbuffer::{Context, Surface};
use std::{
    error::Error,
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, Instant},
};
use tiny_skia::Pixmap;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{Key, ModifiersState, NamedKey},
    window::{CursorIcon, Window, WindowId},
};
use yami_render::native::{NativeDocument, TextEngine};

pub fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    if args.len() > 1 {
        return Err("--desktop accepts at most one yami:// or ynps:// address, localhost ynp:// address or built-in local:welcome/local:about".into());
    }
    let event_loop = EventLoop::<()>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = proxy.send_event(());
    });
    let loader = Loader::new(Arc::clone(&wake))?;
    let mut app = Desktop::new(loader);
    app.discovery = crate::discovery::Discovery::new(wake);
    if let Some(address) = args.first() {
        let _ = app.browser.navigate(address, &app.loader);
    }
    eprintln!("{}\n{}", ynp::WARNING, ynp_tls::WARNING);
    event_loop.run_app(&mut app)?;
    if let Some(error) = app.fatal {
        return Err(error.into());
    }
    Ok(())
}

/// A small Unicode-safe single-line editor shared by the two address fields.
#[derive(Default)]
struct Editor {
    value: String,
    caret: usize,
    selected: bool,
    focus: Option<()>,
}
impl Editor {
    fn activate(&mut self, value: &str) {
        self.value = value.into();
        self.caret = self.value.len();
        self.selected = true;
        self.focus = Some(());
    }
    fn replace(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        let base = if self.selected { 0 } else { self.value.len() };
        if base + text.len() > 4096 {
            return;
        }
        if self.selected {
            self.value.clear();
            self.caret = 0;
            self.selected = false;
        }
        self.value.insert_str(self.caret, &text);
        self.caret += text.len();
    }
    fn previous(&self) -> usize {
        self.value[..self.caret]
            .char_indices()
            .last()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.caret
            + self.value[self.caret..]
                .chars()
                .next()
                .map_or(0, char::len_utf8)
    }
    fn backspace(&mut self) {
        if self.selected {
            self.value.clear();
            self.caret = 0;
            self.selected = false;
        } else if self.caret > 0 {
            let p = self.previous();
            self.value.replace_range(p..self.caret, "");
            self.caret = p;
        }
    }
    fn delete(&mut self) {
        if self.selected {
            self.backspace();
        } else if self.caret < self.value.len() {
            let n = self.next();
            self.value.replace_range(self.caret..n, "");
        }
    }
}
struct CachedDocument {
    id: u64,
    generation: u64,
    width: u32,
    scale: u64,
    document: Result<NativeDocument, String>,
}
struct Desktop {
    window: Option<Arc<Window>>,
    _context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    frame: Option<Pixmap>,
    text: TextEngine,
    browser: Browser,
    loader: Loader,
    discovery: crate::discovery::Discovery,
    scene: Scene,
    editor: Editor,
    mouse: (f32, f32),
    modifiers: ModifiersState,
    keyboard_focus: Option<usize>,
    native: Option<CachedDocument>,
    scroll_drag: bool,
    last: Instant,
    animation: bool,
    fatal: Option<String>,
    clipboard: Option<arboard::Clipboard>,
    composing: bool,
    /// Logical size and scale used when rendering without a window (tests).
    headless: Option<(f32, f32, f32)>,
}
impl Desktop {
    fn new(loader: Loader) -> Self {
        Self {
            window: None,
            _context: None,
            surface: None,
            frame: None,
            text: TextEngine::new(),
            browser: Browser::new(),
            loader,
            discovery: crate::discovery::Discovery::new(Arc::new(|| {})),
            scene: Scene::default(),
            editor: Editor::default(),
            mouse: (-1.0, -1.0),
            modifiers: ModifiersState::empty(),
            keyboard_focus: None,
            native: None,
            scroll_drag: false,
            last: Instant::now(),
            animation: false,
            fatal: None,
            clipboard: None,
            composing: false,
            headless: None,
        }
    }
    fn redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
    fn size(&self) -> (f32, f32, f32) {
        if let Some(size) = self.headless {
            return size;
        }
        let window = self.window.as_ref().unwrap();
        let scale = window.scale_factor() as f32;
        let size = window.inner_size();
        (size.width as f32 / scale, size.height as f32 / scale, scale)
    }
    fn viewport(&self) -> Rect {
        let (w, h, _) = self.size();
        chrome::viewport(w, h)
    }
    fn clear_focus(&mut self) {
        self.composing = false;
        self.editor.focus = None;
        self.keyboard_focus = None;
    }
    fn commit(&mut self) {
        let value = if self.editor.focus.is_some() {
            self.editor.value.clone()
        } else {
            self.browser.active().address.clone()
        };
        let _ = self.browser.navigate(&value, &self.loader);
        self.clear_focus();
        self.redraw();
    }
    fn act(&mut self, action: Action) {
        match action {
            Action::FindLocal => {
                self.discovery.toggle();
                self.clear_focus();
            }
            Action::SelectLan => self.discovery.choose_next_interface(),
            Action::SearchLan => self.discovery.search(),
            Action::LocalNode(index) => {
                if index < self.discovery.results.len() {
                    self.discovery.selected = Some(index);
                }
            }
            Action::OpenLocal => {
                if let Some(url) = self.discovery.selected_url() {
                    let _ = self.browser.navigate(&url, &self.loader);
                    self.discovery.open = false;
                    self.clear_focus();
                }
            }
            Action::LocalPage(next) => {
                if next && (self.discovery.page + 1) * 4 < self.discovery.results.len() {
                    self.discovery.page += 1;
                } else if !next {
                    self.discovery.page = self.discovery.page.saturating_sub(1);
                }
            }
            Action::DiscoveryPanel => {}
            Action::Address => {
                self.composing = false;
                self.editor.activate(&self.browser.active().address);
                self.keyboard_focus = None;
            }
            Action::NewTab => {
                if self.browser.open_tab() {
                    self.clear_focus();
                    self.editor.activate("");
                }
            }
            Action::Close(index) => {
                self.browser.close_tab(index);
                self.clear_focus();
            }
            Action::Tab(index) => {
                self.browser.select(index);
                self.clear_focus();
            }
            Action::Back => {
                self.browser.back(&self.loader);
                self.clear_focus();
            }
            Action::Forward => {
                self.browser.forward(&self.loader);
                self.clear_focus();
            }
            Action::Reload => {
                self.browser.reload(&self.loader);
                self.clear_focus();
            }
            Action::Home => {
                self.browser.navigate_location(Location::Home, &self.loader);
                self.clear_focus();
            }
            Action::Navigate(address) => {
                let _ = self.browser.navigate(address, &self.loader);
                self.clear_focus();
            }
            Action::Scrollbar => {
                self.scroll_drag = true;
                self.scroll_to_mouse();
            }
        }
        self.redraw();
    }
    fn current_layout(&self) -> Option<&NativeDocument> {
        let tab = self.browser.active();
        if !matches!(tab.state, PageState::Ready(_)) {
            return None;
        }
        let (_, _, scale) = self.size();
        let width = (self.viewport().w * scale).round() as u32;
        self.native
            .as_ref()
            .filter(|cache| {
                cache.id == tab.id
                    && cache.generation == tab.generation
                    && cache.width == width
                    && cache.scale == (scale as f64).to_bits()
            })
            .and_then(|cache| cache.document.as_ref().ok())
    }
    fn scroll_limit(&self) -> f64 {
        let (_, _, scale) = self.size();
        let height = self.viewport().h;
        self.current_layout()
            .map_or(0.0, |doc| (f64::from(doc.height / scale - height)).max(0.0))
    }
    fn scroll_to_mouse(&mut self) {
        let area = self.viewport();
        let limit = self.scroll_limit();
        let fraction = ((self.mouse.1 - area.y) / area.h).clamp(0.0, 1.0);
        let tab = self.browser.active_mut();
        tab.target_scroll = f64::from(fraction) * limit;
        tab.scroll = tab.target_scroll;
    }
    fn link(&self) -> Option<String> {
        let (width, height, _) = self.size();
        if self.discovery.open
            && chrome::discovery_rect(width, height).contains(self.mouse.0, self.mouse.1)
        {
            return None;
        }
        let area = self.viewport();
        if !area.contains(self.mouse.0, self.mouse.1) {
            return None;
        }
        let (_, _, scale) = self.size();
        self.current_layout()
            .and_then(|doc| {
                doc.link_at(
                    (self.mouse.0 - area.x) * scale,
                    (self.mouse.1 - area.y + self.browser.active().scroll as f32) * scale,
                )
            })
            .map(str::to_owned)
    }
    fn render(&mut self) -> Result<(), Box<dyn Error>> {
        let window = self.window.as_ref().unwrap();
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            self.animation = false;
            self.last = Instant::now();
            return Ok(());
        }
        if u64::from(size.width) * u64::from(size.height) > 24_000_000 {
            return Err(
                "Window framebuffer exceeds 24 megapixels; reduce display resolution".into(),
            );
        }
        let mut pix = match self.frame.take() {
            Some(p) if p.width() == size.width && p.height() == size.height => p,
            _ => {
                Pixmap::new(size.width, size.height).ok_or("Cannot allocate native framebuffer")?
            }
        };
        self.compose(&mut pix);
        let surface = self.surface.as_mut().unwrap();
        surface.resize(
            NonZeroU32::new(size.width).unwrap(),
            NonZeroU32::new(size.height).unwrap(),
        )?;
        let mut buffer = surface.buffer_mut()?;
        for (out, pixel) in buffer.iter_mut().zip(pix.pixels()) {
            *out = (u32::from(pixel.red()) << 16)
                | (u32::from(pixel.green()) << 8)
                | u32::from(pixel.blue());
        }
        buffer.present()?;
        self.frame = Some(pix);
        if let Some(window) = &self.window {
            window.set_title(&format!("{} — Yami Browser", self.browser.active().title()));
        }
        Ok(())
    }
    /// Paints the whole interface into `pix` and records the hit scene.
    fn compose(&mut self, pix: &mut Pixmap) {
        let (w, h, scale) = self.size();
        // Refresh layout before using a scroll limit from an active tab. A
        // cached short page must never reset another tab's saved scroll.
        if let PageState::Ready(page) = &self.browser.active().state {
            let tab = self.browser.active();
            let doc_width = (chrome::viewport(w, h).w * scale).round() as u32;
            let valid = self.native.as_ref().is_some_and(|cache| {
                cache.id == tab.id
                    && cache.generation == tab.generation
                    && cache.width == doc_width
                    && cache.scale == (scale as f64).to_bits()
            });
            if !valid {
                self.native = Some(CachedDocument {
                    id: tab.id,
                    generation: tab.generation,
                    width: doc_width,
                    scale: (scale as f64).to_bits(),
                    document: NativeDocument::layout(
                        &page.document,
                        &page.style,
                        doc_width as f32,
                        scale,
                        &mut self.text,
                    )
                    .map_err(|e| e.to_string()),
                });
            }
        }
        let elapsed = self.last.elapsed().as_secs_f64().min(0.1);
        self.last = Instant::now();
        let limit = self.scroll_limit();
        let tab = self.browser.active_mut();
        tab.target_scroll = tab.target_scroll.clamp(0.0, limit);
        tab.scroll += (tab.target_scroll - tab.scroll) * (1.0 - (-elapsed * 18.0).exp());
        if (tab.target_scroll - tab.scroll).abs() < 0.2 {
            tab.scroll = tab.target_scroll;
        }
        self.animation = (tab.target_scroll - tab.scroll).abs() > 0.2;
        let value = if self.editor.focus.is_some() {
            &self.editor.value
        } else {
            &self.browser.active().address
        };
        let focus = self
            .keyboard_focus
            .and_then(|i| self.scene.hits.get(i))
            .map(|hit| hit.action.clone());
        let ui = ChromeState {
            width: w,
            height: h,
            mouse: self.mouse,
            editing: self.editor.focus.is_some(),
            value,
            selection: self.editor.selected,
            caret: self.editor.caret,
            focus: focus.as_ref(),
        };
        let viewport = self.viewport();
        let mut c = Canvas::new(pix, &mut self.text, scale);
        let mut scene = chrome::paint(&mut c, &self.browser, &ui);
        match &self.browser.active().state {
            PageState::Home => chrome::home(&mut c, &mut scene, &ui),
            PageState::Loading => {
                chrome::document_viewport(&mut c, &mut scene, &ui);
                let verifying = matches!(
                    self.browser.active().location(),
                    Location::Ynps(_) | Location::Yami(_)
                );
                c.text(
                    if verifying {
                        "Verifying node identity…"
                    } else {
                        "Opening document…"
                    },
                    viewport.x + viewport.w / 2.0 - 60.0,
                    viewport.y + viewport.h / 2.0 - 10.0,
                    200.0,
                    13.0,
                    INK,
                    false,
                );
            }
            PageState::Error(message) => {
                chrome::document_viewport(&mut c, &mut scene, &ui);
                let x = viewport.x + 40.0;
                c.text(
                    "Could not open page",
                    x,
                    viewport.y + 40.0,
                    viewport.w - 80.0,
                    16.0,
                    INK,
                    true,
                );
                let mut y = viewport.y + 76.0;
                let chars: Vec<char> = message.chars().take(700).collect();
                for part in chars.chunks(90) {
                    let line: String = part.iter().collect();
                    c.text(&line, x, y, viewport.w - 80.0, 13.0, INK, false);
                    y += 21.0;
                }
            }
            PageState::Ready(_) => {
                let tab = self.browser.active();
                // The YLang document paints its own paper edge to edge in the
                // viewport, so layout width, drawing and link hit-tests agree.
                match self.native.as_ref().map(|cache| &cache.document) {
                    Some(Ok(doc)) => {
                        doc.draw(
                            c.pix,
                            viewport.x * scale,
                            viewport.y * scale,
                            tab.scroll as f32 * scale,
                            viewport.h * scale,
                            c.text,
                        );
                        if doc.height > viewport.h * scale {
                            let bar = Rect::new(
                                viewport.x + viewport.w - 12.0,
                                viewport.y + 10.0,
                                5.0,
                                viewport.h - 20.0,
                            );
                            let thumb_h = (bar.h * viewport.h * scale / doc.height).max(24.0);
                            let max = (doc.height / scale - viewport.h).max(1.0);
                            let yy = bar.y
                                + (bar.h - thumb_h) * (tab.scroll as f32 / max).clamp(0.0, 1.0);
                            c.round(
                                Rect::new(bar.x, yy, bar.w, thumb_h),
                                2.5,
                                [117, 80, 93, 170],
                                [117, 80, 93, 170],
                                None,
                            );
                            scene.hits.push(chrome::Hit {
                                rect: Rect::new(bar.x - 5.0, bar.y, 15.0, bar.h),
                                action: Action::Scrollbar,
                            });
                        }
                    }
                    Some(Err(error)) => {
                        chrome::document_viewport(&mut c, &mut scene, &ui);
                        c.text(
                            error,
                            viewport.x + 40.0,
                            viewport.y + 40.0,
                            viewport.w - 80.0,
                            13.0,
                            INK,
                            false,
                        );
                    }
                    None => chrome::document_viewport(&mut c, &mut scene, &ui),
                }
                chrome::round_viewport(&mut c, &ui);
            }
        }
        chrome::discovery_panel(&mut c, &mut scene, &ui, &self.discovery);
        self.scene = scene;
    }
    fn keyboard(&mut self, key: &Key, text: Option<&str>, event_loop: &ActiveEventLoop) {
        // Windows exposes AltGr as Ctrl+Alt. Preserve printable input from
        // European keyboard layouts (including IPv6 brackets) in the editor.
        let altgr = self.modifiers.control_key() && self.modifiers.alt_key();
        let command = (self.modifiers.control_key() || self.modifiers.super_key()) && !altgr;
        if command {
            if let Key::Character(ch) = key {
                match ch.to_lowercase().as_str() {
                    "l" => {
                        self.act(Action::Address);
                        return;
                    }
                    "t" => {
                        self.act(Action::NewTab);
                        return;
                    }
                    "w" => {
                        self.act(Action::Close(self.browser.active));
                        return;
                    }
                    "r" => {
                        self.act(Action::Reload);
                        return;
                    }
                    "a" if self.editor.focus.is_some() => {
                        self.editor.selected = true;
                        self.redraw();
                        return;
                    }
                    "c" | "x" if self.editor.focus.is_some() => {
                        if self.editor.selected {
                            if self.clipboard.is_none() {
                                self.clipboard = arboard::Clipboard::new().ok();
                            }
                            if let Some(clipboard) = &mut self.clipboard {
                                let _ = clipboard.set_text(self.editor.value.clone());
                            }
                            if ch.eq_ignore_ascii_case("x") {
                                self.editor.backspace();
                            }
                        }
                        self.redraw();
                        return;
                    }
                    "v" if self.editor.focus.is_some() => {
                        if self.clipboard.is_none() {
                            self.clipboard = arboard::Clipboard::new().ok();
                        }
                        if let Some(clipboard) = &mut self.clipboard {
                            if let Ok(value) = clipboard.get_text() {
                                self.editor.replace(&value);
                            }
                        }
                        self.redraw();
                        return;
                    }
                    _ => {}
                }
            }
        }
        if self.modifiers.alt_key() {
            match key {
                Key::Named(NamedKey::ArrowLeft) => {
                    self.act(Action::Back);
                    return;
                }
                Key::Named(NamedKey::ArrowRight) => {
                    self.act(Action::Forward);
                    return;
                }
                _ => {}
            }
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.discovery.open = false;
                self.clear_focus();
            }
            Key::Named(NamedKey::Tab) if command => {
                let n = self.browser.tabs.len();
                let next = if self.modifiers.shift_key() {
                    (self.browser.active + n - 1) % n
                } else {
                    (self.browser.active + 1) % n
                };
                self.act(Action::Tab(next));
            }
            Key::Named(NamedKey::Tab) => {
                self.editor.focus = None;
                let n = self.scene.hits.len();
                if n > 0 {
                    self.keyboard_focus = Some(match self.keyboard_focus {
                        None => 0,
                        Some(i) if self.modifiers.shift_key() => (i + n - 1) % n,
                        Some(i) => (i + 1) % n,
                    });
                }
            }
            Key::Named(NamedKey::Enter) => {
                if self.editor.focus.is_some() {
                    self.commit();
                } else if let Some(hit) = self.keyboard_focus.and_then(|i| self.scene.hits.get(i)) {
                    self.act(hit.action.clone());
                }
            }
            Key::Named(NamedKey::Backspace) if self.editor.focus.is_some() => {
                self.editor.backspace()
            }
            Key::Named(NamedKey::Delete) if self.editor.focus.is_some() => self.editor.delete(),
            Key::Named(NamedKey::ArrowLeft) if self.editor.focus.is_some() => {
                self.editor.caret = if self.editor.selected {
                    0
                } else {
                    self.editor.previous()
                };
                self.editor.selected = false;
            }
            Key::Named(NamedKey::ArrowRight) if self.editor.focus.is_some() => {
                self.editor.caret = if self.editor.selected {
                    self.editor.value.len()
                } else {
                    self.editor.next()
                };
                self.editor.selected = false;
            }
            Key::Named(NamedKey::Home) if self.editor.focus.is_some() => {
                self.editor.caret = 0;
                self.editor.selected = false;
            }
            Key::Named(NamedKey::End) if self.editor.focus.is_some() => {
                self.editor.caret = self.editor.value.len();
                self.editor.selected = false;
            }
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::ArrowDown)
                if self.editor.focus.is_none() =>
            {
                let delta = if key == &Key::Named(NamedKey::PageDown) {
                    self.viewport().h * 0.8
                } else {
                    50.0
                };
                let limit = self.scroll_limit();
                let tab = self.browser.active_mut();
                tab.target_scroll = (tab.target_scroll + f64::from(delta)).min(limit);
            }
            Key::Named(NamedKey::PageUp) | Key::Named(NamedKey::ArrowUp)
                if self.editor.focus.is_none() =>
            {
                let delta = if key == &Key::Named(NamedKey::PageUp) {
                    self.viewport().h * 0.8
                } else {
                    50.0
                };
                let tab = self.browser.active_mut();
                tab.target_scroll = (tab.target_scroll - f64::from(delta)).max(0.0);
            }
            Key::Named(NamedKey::F5) => self.act(Action::Reload),
            Key::Named(NamedKey::F4) if self.modifiers.alt_key() => event_loop.exit(),
            _ if self.editor.focus.is_some()
                && !command
                && (!self.modifiers.alt_key() || altgr) =>
            {
                if let Some(value) = text {
                    self.editor.replace(value);
                }
            }
            _ => {}
        }
        self.redraw();
    }
}
impl ApplicationHandler<()> for Desktop {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let result = (|| -> Result<(), Box<dyn Error>> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title("Yami Browser")
                        .with_inner_size(LogicalSize::new(1440.0, 900.0))
                        .with_min_inner_size(LogicalSize::new(900.0, 600.0)),
                )?,
            );
            window.set_ime_allowed(true);
            let context = Context::new(window.clone())?;
            let surface = Surface::new(&context, window.clone())?;
            self.window = Some(window);
            self._context = Some(context);
            self.surface = Some(surface);
            self.redraw();
            Ok(())
        })();
        if let Err(error) = result {
            self.fatal = Some(error.to_string());
            event_loop.exit();
        }
    }
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        let discovery_changed = self.discovery.poll();
        if self.browser.poll(&self.loader) || discovery_changed {
            self.redraw();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => self.redraw(),
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render() {
                    self.fatal = Some(error.to_string());
                    event_loop.exit();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed && !self.composing =>
            {
                self.keyboard(&event.logical_key, event.text.as_deref(), event_loop)
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.composing = false;
            }
            WindowEvent::Occluded(true) => {
                self.animation = false;
            }
            WindowEvent::Occluded(false) => {
                self.last = Instant::now();
                self.redraw();
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => {
                self.composing = !text.is_empty();
            }
            WindowEvent::Ime(Ime::Commit(text)) if self.editor.focus.is_some() => {
                self.composing = false;
                self.editor.replace(&text);
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.window.as_ref().unwrap().scale_factor() as f32;
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                if self.scroll_drag {
                    self.scroll_to_mouse();
                }
                let cursor = match self.scene.hit(self.mouse.0, self.mouse.1) {
                    Some(Action::Address) => CursorIcon::Text,
                    Some(_) => CursorIcon::Pointer,
                    None if self.link().is_some() => CursorIcon::Pointer,
                    _ => CursorIcon::Default,
                };
                self.window.as_ref().unwrap().set_cursor(cursor);
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse = (-1.0, -1.0);
                self.redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(action) = self.scene.hit(self.mouse.0, self.mouse.1) {
                    self.act(action);
                } else if let Some(target) = self.link() {
                    let _ = self.browser.follow(&target, &self.loader);
                    self.clear_focus();
                    self.redraw();
                } else {
                    self.clear_focus();
                    self.redraw();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.scroll_drag = false,
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y) * 54.0,
                    MouseScrollDelta::PixelDelta(p) => {
                        p.y / self.window.as_ref().unwrap().scale_factor()
                    }
                };
                let limit = self.scroll_limit();
                let tab = self.browser.active_mut();
                tab.target_scroll = (tab.target_scroll - amount).clamp(0.0, limit);
                self.redraw();
            }
            WindowEvent::Focused(false) => {
                self.modifiers = ModifiersState::empty();
                self.scroll_drag = false;
                self.composing = false;
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.animation {
            let deadline = self.last + Duration::from_millis(16);
            if Instant::now() >= deadline {
                self.redraw();
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editor_handles_unicode_selection_and_bounds() {
        let mut e = Editor::default();
        e.activate("café");
        e.replace("ynp://");
        assert_eq!(e.value, "ynp://");
        e.replace("é");
        e.backspace();
        assert_eq!(e.value, "ynp://");
        e.caret = 0;
        e.delete();
        assert_eq!(e.value, "np://");
        e.replace("\n\tbad\u{1b}");
        assert_eq!(e.value, "badnp://");
        e.replace(&"x".repeat(4097));
        assert_eq!(e.value, "badnp://");
    }

    fn headless(w: f32, h: f32) -> Desktop {
        let mut app = Desktop::new(Loader::new(Arc::new(|| {})).unwrap());
        app.headless = Some((w, h, 1.0));
        app
    }
    fn settle(app: &mut Desktop) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while matches!(app.browser.active().state, PageState::Loading) {
            app.browser.poll(&app.loader);
            assert!(Instant::now() < deadline, "loader did not complete");
            std::thread::yield_now();
        }
    }
    fn frame(app: &mut Desktop) -> Pixmap {
        let (w, h, _) = app.size();
        let mut pix = Pixmap::new(w as u32, h as u32).unwrap();
        app.compose(&mut pix);
        pix
    }

    #[test]
    fn discovery_selection_is_untrusted_and_does_not_navigate_or_replace_tabs() {
        let mut app = headless(1440.0, 900.0);
        app.browser.navigate("local:welcome", &app.loader).unwrap();
        settle(&mut app);
        app.browser.open_tab();
        let original = app.browser.active().address.clone();
        let id = *yami_identity::NodeIdentity::generate().unwrap().id();
        let advert =
            ynp_discovery::Advertisement::new(id, "192.168.1.20:7443".parse().unwrap(), "home")
                .unwrap();
        app.discovery.open = true;
        app.discovery.results = vec![advert.clone()];
        let pix = frame(&mut app);
        assert!(app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::FindLocal));
        let hit = app
            .scene
            .hits
            .iter()
            .find(|hit| hit.action == Action::LocalNode(0))
            .unwrap();
        let action = app.scene.hit(hit.rect.x + 5.0, hit.rect.y + 5.0).unwrap();
        app.act(action);
        frame(&mut app);
        assert_eq!(app.discovery.selected_url(), Some(advert.url()));
        assert_eq!(app.browser.active().address, original);
        assert_eq!(app.browser.tabs.len(), 2);
        assert_eq!(app.browser.tabs[0].address, "local:welcome");
        assert!(app.browser.active().security().is_none());
        assert!(app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::OpenLocal));
        assert!(pix.width() > 0);
        app.discovery.choose_next_interface(); // no interface selected: no traffic
        app.act(Action::FindLocal);
        assert!(!app.discovery.open);
        assert_eq!(app.browser.active().address, original);
    }

    #[test]
    #[ignore = "uses selected private interface and multicast; set YAMINET_TEST_LAN_ADDRESS"]
    fn discovered_node_opens_via_verified_tls_and_preserves_navigation() {
        use ynp_discovery::{Advertisement, Config, Responder};
        let ip: std::net::Ipv4Addr = std::env::var("YAMINET_TEST_LAN_ADDRESS")
            .expect("select a private LAN interface")
            .parse()
            .unwrap();
        let interface = ynp_discovery::interface(ip).unwrap();
        let config = Config::from_env().unwrap();
        let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener = ynp_tls::TlsListener::bind_lan((ip, 0).into(), identity).unwrap();
        let std::net::SocketAddr::V4(endpoint) = listener.local_addr().unwrap() else {
            unreachable!()
        };
        let mut store = yami_node::Store::default();
        store
            .insert(ynp::DocumentData {
                key: "home".into(),
                source: "@page 0.1\n  title Discovered home\n= Discovered home\n".into(),
                style: String::new(),
            })
            .unwrap();
        let server = std::thread::spawn(move || {
            // Two successful loads (open and back), then a forged claim.
            for _ in 0..3 {
                let _ = yami_node::serve_once(&listener, &store);
            }
        });
        let responder = Responder::start(
            interface.clone(),
            config,
            Advertisement::new(id, endpoint, "home").unwrap(),
        )
        .unwrap();
        let mut app = headless(1440.0, 900.0);
        app.browser.navigate("local:welcome", &app.loader).unwrap();
        settle(&mut app);
        app.browser.open_tab();
        app.discovery.open = true;
        app.discovery.interfaces = vec![interface];
        app.act(Action::SelectLan);
        app.act(Action::SearchLan);
        assert!(app.discovery.searching());
        // A repeated search click cannot replace the current worker.
        app.act(Action::SearchLan);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.discovery.searching() {
            assert!(matches!(app.browser.active().state, PageState::Home));
            app.discovery.poll();
            assert!(
                Instant::now() < deadline,
                "browser discovery exceeded its deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            app.discovery.selected.is_none(),
            "responses never select a node"
        );
        let index = app
            .discovery
            .results
            .iter()
            .position(|node| node.id() == id)
            .expect("browser discovers the node");
        app.act(Action::LocalNode(index));
        assert!(matches!(app.browser.active().state, PageState::Home));
        app.act(Action::OpenLocal);
        settle(&mut app);
        assert_eq!(
            app.browser.active().security(),
            Some(&crate::model::Security::Verified(id))
        );
        assert_eq!(app.browser.active().title(), "Discovered home");
        assert_eq!(app.browser.tabs.len(), 2);
        app.act(Action::Navigate("local:about"));
        settle(&mut app);
        app.act(Action::Back);
        settle(&mut app);
        assert_eq!(app.browser.active().title(), "Discovered home");
        let forged = *yami_identity::NodeIdentity::generate().unwrap().id();
        app.discovery.results = vec![Advertisement::new(forged, endpoint, "home").unwrap()];
        app.discovery.selected = Some(0);
        app.act(Action::OpenLocal);
        settle(&mut app);
        assert!(
            matches!(&app.browser.active().state, PageState::Error(error) if error.contains("Authentication failed"))
        );
        assert!(app.browser.active().security().is_none());
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        assert_eq!(app.browser.active().title(), "Welcome to YamiNet");
        drop(responder);
        server.join().unwrap();
    }

    #[test]
    #[ignore = "writes a manual visual QA screenshot under target/screenshots"]
    fn capture_discovery() {
        let mut app = headless(1440.0, 900.0);
        app.discovery.open = true;
        app.discovery.results = (20..23)
            .map(|host| {
                ynp_discovery::Advertisement::new(
                    *yami_identity::NodeIdentity::generate().unwrap().id(),
                    format!("192.168.1.{host}:7443").parse().unwrap(),
                    "home",
                )
                .unwrap()
            })
            .collect();
        app.discovery.selected = Some(0);
        app.discovery.status = "3 untrusted node(s). Select one to inspect.".into();
        let output = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/screenshots"
        ));
        std::fs::create_dir_all(output).unwrap();
        frame(&mut app)
            .save_png(output.join("discovery.png"))
            .unwrap();
    }

    #[test]
    fn viewport_dominates_at_1440_and_sidebar_actions_navigate() {
        let mut app = headless(1440.0, 900.0);
        let view = app.viewport();
        assert!(view.w / 1440.0 >= 0.8, "viewport is {}px wide", view.w);
        frame(&mut app);
        let pin = app
            .scene
            .hits
            .iter()
            .find(|hit| hit.action == Action::Navigate("local:welcome"))
            .expect("welcome pin")
            .rect;
        app.mouse = (pin.x + 2.0, pin.y + 2.0);
        app.act(app.scene.hit(app.mouse.0, app.mouse.1).unwrap());
        settle(&mut app);
        assert!(matches!(app.browser.active().state, PageState::Ready(_)));
        frame(&mut app);
        assert!(
            app.current_layout().is_some(),
            "document laid out at viewport width"
        );
        app.act(Action::Home);
        assert!(matches!(app.browser.active().state, PageState::Home));
    }

    /// A long page over TLS: the sidebar shows the verified node, the page
    /// scrolls, and a plaintext page in another tab gets no verified marking.
    #[test]
    fn verified_page_shows_the_node_and_scrolls() {
        let dir = std::env::temp_dir().join(format!("yaminet-desktop-tls-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut source = String::from("@page 0.1\n  title Long secure page\n= Long secure page\n");
        for line in 0..120 {
            source.push_str(&format!(
                "Paragraph {line} of a document fetched over TLS 1.3.\n"
            ));
        }
        std::fs::write(dir.join("long.ylang"), source).unwrap();
        let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
        let address = listener.local_addr().unwrap();
        let site = yami_node::Site::open(&dir).unwrap();
        std::thread::spawn(move || yami_node::serve_concurrently(&listener, &site, 2, &|_| {}));

        let mut app = headless(1440.0, 900.0);
        frame(&mut app);
        let pin_y = |app: &Desktop| {
            app.scene
                .hits
                .iter()
                .find(|hit| hit.action == Action::Navigate("local:welcome"))
                .unwrap()
                .rect
                .y
        };
        let home_pins = pin_y(&app);
        app.browser
            .navigate(&ynp_tls::format_url(&id, address, "long"), &app.loader)
            .unwrap();
        settle(&mut app);
        assert_eq!(
            app.browser.active().security(),
            Some(&crate::model::Security::Verified(id))
        );
        frame(&mut app);
        // The identity block sits between the address field and the pins.
        assert!(pin_y(&app) > home_pins + 40.0);
        assert!(app.scroll_limit() > 0.0);
        app.browser.active_mut().target_scroll = 300.0;
        for _ in 0..30 {
            frame(&mut app);
        }
        assert!(app.browser.active().scroll > 250.0);
        // Reload keeps the verified marking; a failure removes it.
        app.act(Action::Reload);
        settle(&mut app);
        assert!(app.browser.active().security().is_some());
        let other = *yami_identity::NodeIdentity::generate().unwrap().id();
        app.browser
            .navigate(&ynp_tls::format_url(&other, address, "long"), &app.loader)
            .unwrap();
        settle(&mut app);
        assert!(matches!(app.browser.active().state, PageState::Error(_)));
        frame(&mut app);
        assert_eq!(pin_y(&app), home_pins);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Writes a 1440×900 capture of a page fetched over TLS, showing the
    /// verified node in the sidebar. Run with `-- --ignored capture_verified`.
    #[test]
    #[ignore = "writes target/screenshots/desktop-verified.png"]
    fn capture_verified() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let identity = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
        let id = *identity.id();
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity).unwrap();
        let address = listener.local_addr().unwrap();
        let site = yami_node::Site::open(root.join("sites/alice")).unwrap();
        std::thread::spawn(move || yami_node::serve_concurrently(&listener, &site, 2, &|_| {}));
        let dir = root.join("target/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = headless(1440.0, 900.0);
        app.browser
            .navigate(&ynp_tls::format_url(&id, address, "home"), &app.loader)
            .unwrap();
        settle(&mut app);
        frame(&mut app);
        frame(&mut app)
            .save_png(dir.join("desktop-verified.png"))
            .unwrap();
    }

    /// Writes 1440×900 captures of the start page and a YLang document to
    /// `target/screenshots`. Run with `cargo test -p yami-browser -- --ignored`.
    #[test]
    #[ignore]
    fn capture_screenshots() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = headless(1440.0, 900.0);
        frame(&mut app)
            .save_png(dir.join("desktop-home.png"))
            .unwrap();
        app.browser.navigate("local:welcome", &app.loader).unwrap();
        settle(&mut app);
        app.browser.open_tab();
        app.browser.navigate("local:about", &app.loader).unwrap();
        settle(&mut app);
        app.browser.select(0);
        frame(&mut app);
        frame(&mut app)
            .save_png(dir.join("desktop-document.png"))
            .unwrap();
    }

    /// Point at a rendered link the way a user would, then click it.
    fn click_link(app: &mut Desktop, target: &str) {
        frame(app);
        let area = app.viewport();
        let mut y = area.y;
        while y < area.y + area.h {
            let mut x = area.x;
            while x < area.x + area.w {
                app.mouse = (x, y);
                if app.link().as_deref() == Some(target) {
                    app.browser.follow(target, &app.loader).unwrap();
                    settle(app);
                    return;
                }
                x += 6.0;
            }
            y += 4.0;
        }
        panic!("link {target} is not visible");
    }
    /// Serves the sample sites on their documented ports and walks across both
    /// nodes through rendered links. Run with `-- --ignored capture_sites`.
    #[test]
    #[ignore = "binds 127.0.0.1:7301 and 7302 and writes PNGs to target/screenshots"]
    fn capture_sites() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (port, site) in [(7301, "sites/alice"), (7302, "sites/bob")] {
            let listener = ynp::LocalListener::bind(([127, 0, 0, 1], port).into()).unwrap();
            let site = yami_node::Site::open(root.join(site)).unwrap();
            std::thread::spawn(move || yami_node::serve_concurrently(&listener, &site, 2, &|_| {}));
        }
        let dir = root.join("target/screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = headless(1440.0, 900.0);
        app.browser
            .navigate("ynp://127.0.0.1:7301/home", &app.loader)
            .unwrap();
        settle(&mut app);
        frame(&mut app)
            .save_png(dir.join("site-alice-home.png"))
            .unwrap();
        for (target, title, file) in [
            (
                "ynp://127.0.0.1:7301/projects",
                "Alice's Projects",
                "site-alice-projects.png",
            ),
            (
                "ynp://127.0.0.1:7302/home",
                "Bob's Home",
                "site-bob-home.png",
            ),
            (
                "ynp://127.0.0.1:7302/garden",
                "Bob's Garden Log",
                "site-bob-garden.png",
            ),
            (
                "ynp://127.0.0.1:7301/about",
                "About Alice",
                "site-alice-about.png",
            ),
        ] {
            click_link(&mut app, target);
            assert_eq!(app.browser.active().title(), title);
            frame(&mut app).save_png(dir.join(file)).unwrap();
        }
        assert!(app.browser.back(&app.loader));
        settle(&mut app);
        assert_eq!(app.browser.active().title(), "Bob's Garden Log");
    }
}
