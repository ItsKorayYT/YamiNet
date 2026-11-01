//! Native window/event adapter. All network and parsing work lives in Loader.
use crate::{
    chrome::{self, Action, Canvas, ChromeState, Rect, Scene, INK},
    model::{Browser, Loader, Location, PageState},
};
mod smoke;
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

pub fn run(
    args: Vec<String>,
    relay_enabled: Option<bool>,
    resolver: Option<Arc<yami_browser::native::Resolver>>,
    diagnostic_dir: Option<std::path::PathBuf>,
    smoke_output: Option<std::path::PathBuf>,
    game_path: Option<std::path::PathBuf>,
) -> Result<(), Box<dyn Error>> {
    if args.len() > 1 {
        return Err("--desktop accepts at most one yami:// or ynps:// address, localhost ynp:// address or built-in local:welcome/local:about".into());
    }
    let event_loop = EventLoop::<()>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let _ = proxy.send_event(());
    });
    let explicit_network = resolver.is_some()
        || relay_enabled.is_some()
        || std::env::var_os("YAMINET_BROWSER_DHT_NETWORK").is_some()
        || std::env::var_os("YAMINET_BROWSER_RELAYS").is_some();
    let loader = if let Some(resolver) = resolver {
        Loader::with_resolver(Arc::clone(&wake), resolver)?
    } else {
        Loader::new(Arc::clone(&wake))?
    };
    if let Some(enabled) = relay_enabled {
        loader.set_relay_enabled(enabled);
    }
    let mut app = Desktop::new(loader);
    match yami_browser::library::Library::default_directory()
        .and_then(yami_browser::library::Library::open)
    {
        Ok(library) => {
            app.settings = library.settings();
            app.browser.library = Some(library);
        }
        Err(error) => {
            eprintln!("Browser library unavailable: {error}");
            app.browser.library_error = Some(error.to_string());
        }
    }
    if explicit_network {
        app.settings.local_only = false;
        app.settings.relays = app.loader.relay_enabled();
    }
    if let Some(enabled) = relay_enabled {
        app.settings.relays = enabled;
    }
    app.loader.set_local_only(app.settings.local_only);
    app.loader.set_relay_enabled(app.settings.relays);
    if let Some(dir) = diagnostic_dir {
        app.log = Some(yami_browser::diagnostics::Log::open(&dir)?);
        app.record("startup");
    }
    if let Some(output) = smoke_output {
        app.settings.local_only = true;
        app.apply_local_only(true);
        app.loader.set_relay_enabled(false);
        app.smoke = Some(smoke::Smoke::new(output, app.settings.onboarded));
    }
    app.discover.set_wake(wake.clone());
    app.game_wake = Arc::clone(&wake);
    app.discovery = crate::discovery::Discovery::new(wake);
    if let Some(address) = args.first() {
        let _ = app.browser.navigate(address, &app.loader);
    }
    if let Some(path) = game_path {
        app.browser.open_game(path, Arc::clone(&app.game_wake))?;
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
    anchor: Option<usize>,
    focus: Option<()>,
}
impl Editor {
    fn activate(&mut self, value: &str) {
        self.value = value.into();
        self.caret = self.value.len();
        self.selected = true;
        self.anchor = None;
        self.focus = Some(());
    }
    fn range(&self) -> Option<std::ops::Range<usize>> {
        self.selected.then(|| {
            self.anchor.map_or(0..self.value.len(), |a| {
                a.min(self.caret)..a.max(self.caret)
            })
        })
    }
    fn move_caret(&mut self, to: usize, extend: bool) {
        if extend {
            if !self.selected {
                self.anchor = Some(self.caret);
            }
            self.caret = to;
            self.selected = self.anchor != Some(to);
        } else {
            self.caret = to;
            self.selected = false;
            self.anchor = None;
        }
    }
    fn replace(&mut self, text: &str) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        let range = self.range().unwrap_or(self.caret..self.caret);
        if self.value.len() - range.len() + text.len() > 4096 {
            return;
        }
        self.value.replace_range(range.clone(), &text);
        self.caret = range.start + text.len();
        self.selected = false;
        self.anchor = None;
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
        if let Some(range) = self.range() {
            self.value.replace_range(range.clone(), "");
            self.caret = range.start;
            self.selected = false;
            self.anchor = None;
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
    revision: u64,
    id: u64,
    generation: u64,
    width: u32,
    scale: u64,
    document: Result<NativeDocument, String>,
}
struct Desktop {
    game_wake: Arc<dyn Fn() + Send + Sync>,
    game_open: bool,
    game_immersive: bool,
    game_error: Option<String>,
    game_keys: std::collections::HashSet<winit::keyboard::KeyCode>,
    game_presentation: super::game_view::Presentation,
    smoke: Option<smoke::Smoke>,
    frames: u64,
    log: Option<yami_browser::diagnostics::Log>,
    find: yami_browser::find::Find,
    find_open: bool,
    find_edit: bool,
    find_stamp: Option<(u64, u64, u64, u32, u64)>,
    settings: yami_browser::library::Settings,
    settings_open: bool,
    window: Option<Arc<Window>>,
    _context: Option<Context<Arc<Window>>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    frame: Option<Pixmap>,
    text: TextEngine,
    browser: Browser,
    loader: Loader,
    discovery: crate::discovery::Discovery,
    discover: yami_browser::discover::Discover,
    discover_edit: Option<yami_browser::discover::Field>,
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
    bootstrap_edit: bool,
    library_open: bool,
    library_page: usize,
    bookmark_edit: Option<(Option<usize>, bool)>,
    pressed_link: Option<String>,
    /// Logical size and scale used when rendering without a window (tests).
    headless: Option<(f32, f32, f32)>,
}
impl Desktop {
    fn new(loader: Loader) -> Self {
        Self {
            game_wake: Arc::new(|| {}),
            game_open: false,
            game_immersive: true,
            game_error: None,
            game_keys: std::collections::HashSet::new(),
            game_presentation: super::game_view::Presentation::default(),
            smoke: None,
            frames: 0,
            log: None,
            find: yami_browser::find::Find::default(),
            find_open: false,
            find_edit: false,
            find_stamp: None,
            settings: yami_browser::library::Settings::default(),
            settings_open: false,
            window: None,
            _context: None,
            surface: None,
            frame: None,
            text: TextEngine::new(),
            browser: Browser::new(),
            loader,
            discovery: crate::discovery::Discovery::new(Arc::new(|| {})),
            discover: yami_browser::discover::Discover::new(
                yami_browser::discover::Discover::default_directory(),
                Arc::new(|| {}),
            ),
            discover_edit: None,
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
            bootstrap_edit: false,
            library_open: false,
            library_page: 0,
            bookmark_edit: None,
            pressed_link: None,
            headless: None,
        }
    }
    fn record(&mut self, code: &'static str) {
        if let Some(log) = &mut self.log {
            if log.event(code).is_err() {
                self.log = None;
                eprintln!("Local diagnostic log unavailable; logging disabled");
            }
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
        let mut area = chrome::viewport(w, h);
        if self.find_open {
            // Reserve space so the first match cannot sit behind the search bar.
            area.y += 88.0;
            area.h = (area.h - 88.0).max(1.0);
        }
        area
    }
    fn clear_focus(&mut self) {
        if let Some(field) = self.discover_edit.take() {
            self.discover.set_field(field, self.editor.value.clone());
        }
        self.find_edit = false;
        self.bootstrap_edit = false;
        self.bookmark_edit = None;
        self.composing = false;
        self.editor.focus = None;
        self.keyboard_focus = None;
    }
    fn commit(&mut self) {
        if self.game_open {
            let path = std::path::PathBuf::from(self.editor.value.trim());
            match self.browser.open_game(path, Arc::clone(&self.game_wake)) {
                Ok(()) => {
                    self.game_open = false;
                    self.game_immersive = true;
                    self.clear_focus();
                }
                Err(error) => self.game_error = Some(error),
            }
            self.redraw();
            return;
        }
        if self.discover_edit.is_some() {
            self.clear_focus();
            self.redraw();
            return;
        }
        if self.find_edit {
            let changed = self.find.query != self.editor.value;
            self.update_find();
            if !changed {
                self.find.step(self.modifiers.shift_key());
            }
            self.scroll_to_match();
            self.redraw();
            return;
        }
        let value = if self.editor.focus.is_some() && !self.find_edit {
            self.editor.value.clone()
        } else {
            self.browser.active().address.clone()
        };
        if let Some((index, folder)) = self.bookmark_edit {
            let result = if let Some(index) = index {
                if let Some(library) = &mut self.browser.library {
                    if let Some(b) = library.bookmarks().get(index).cloned() {
                        library.save_bookmark(
                            &b.address,
                            if folder { &b.title } else { &value },
                            if folder { &value } else { &b.folder },
                        )
                    } else {
                        Ok(())
                    }
                } else {
                    Err(std::io::Error::other("Browser library unavailable"))
                }
            } else {
                self.browser.bookmark_current(&value, "")
            };
            if let Err(e) = result {
                self.browser.library_error = Some(e.to_string());
            }
        } else if self.bootstrap_edit {
            if let Err(error) = self.loader.add_bootstrap(&value) {
                self.browser.active_mut().state = PageState::Error(error);
            }
        } else {
            let _ = self.browser.navigate(&value, &self.loader);
        }
        self.clear_focus();
        self.redraw();
    }
    fn act(&mut self, action: Action) {
        self.game_keys.clear();
        if !matches!(
            action,
            Action::LaunchGame | Action::GamePath | Action::DiscoveryPanel
        ) {
            self.game_open = false;
        }
        // Any browser action releases held game keys. Modal controls also pause it.
        if let PageState::Game(game) = &self.browser.active().state {
            game.session.clear_input();
            if matches!(
                action,
                Action::OpenGame
                    | Action::Address
                    | Action::Find
                    | Action::Settings
                    | Action::Library
                    | Action::SaveBookmark
                    | Action::Discover
                    | Action::FindLocal
            ) {
                game.session.pause();
                self.game_immersive = false;
            }
        }
        if matches!(
            &action,
            Action::Address
                | Action::NewTab
                | Action::Home
                | Action::Tab(_)
                | Action::Close(_)
                | Action::Back
                | Action::Forward
                | Action::Reload
                | Action::Navigate(_)
                | Action::Follow(_)
        ) {
            self.discover.open = false;
        }
        if matches!(
            &action,
            Action::Follow(_)
                | Action::Navigate(_)
                | Action::Back
                | Action::Forward
                | Action::Reload
        ) {
            self.record("navigation-start");
        }
        match action {
            Action::OpenGame => {
                self.clear_focus();
                self.game_open = true;
                self.game_error = None;
                self.settings_open = false;
                self.library_open = false;
                self.discover.open = false;
                self.discovery.open = false;
                self.find_open = false;
                self.editor.activate("");
            }
            Action::GamePath => {
                self.editor.focus = Some(());
            }
            Action::LaunchGame => self.commit(),
            Action::GameLayout => {
                self.game_immersive = !self.game_immersive;
                self.browser.pause_active_game();
                self.clear_focus();
            }
            Action::GamePause => {
                if let PageState::Game(game) = &self.browser.active().state {
                    match game.snapshot.status {
                        yami_games::Status::Paused => game.session.resume(),
                        _ => game.session.pause(),
                    }
                }
                self.clear_focus();
            }
            Action::GameReset => {
                if let PageState::Game(game) = &mut self.browser.active_mut().state {
                    if matches!(
                        game.snapshot.status,
                        yami_games::Status::Stopped | yami_games::Status::Failed(_)
                    ) {
                        match game.source.launch(Arc::clone(&self.game_wake)) {
                            Ok(session) => {
                                game.session = session;
                                game.snapshot = game.session.snapshot();
                            }
                            Err(error) => game.snapshot.status = yami_games::Status::Failed(error),
                        }
                    } else {
                        game.session.reset();
                    }
                }
                self.clear_focus();
            }
            Action::GameStop => {
                if let PageState::Game(game) = &self.browser.active().state {
                    game.session.stop();
                }
                self.clear_focus();
            }
            Action::Discover => {
                self.clear_focus();
                self.discover.toggle();
                self.library_open = false;
                self.discovery.open = false;
                self.settings_open = false;
                self.find_open = false;
                self.pressed_link = None;
            }
            Action::DiscoverMode(pages) => {
                self.clear_focus();
                self.discover.set_mode(pages);
            }
            Action::DiscoverCancel => {
                self.clear_focus();
                self.discover.pages.cancel();
            }
            Action::DiscoverView(directories) => {
                self.clear_focus();
                self.discover.directories = directories;
                self.discover.page = 0;
                self.discover.pages.page = 0;
            }
            Action::DiscoverField(field) => {
                self.clear_focus();
                self.editor.activate(self.discover.field(field));
                self.discover_edit = Some(field);
            }
            Action::DiscoverSearch => {
                self.clear_focus();
                let query = self
                    .discover
                    .field(yami_browser::discover::Field::Search)
                    .to_owned();
                self.discover
                    .set_field(yami_browser::discover::Field::Search, query);
            }
            Action::DiscoverAdd => {
                self.clear_focus();
                if self.discover.pages_mode {
                    self.discover.pages.add();
                } else {
                    self.discover.add();
                }
            }
            Action::DiscoverSource(name) => {
                self.clear_focus();
                if self.discover.pages_mode {
                    self.discover.pages.selected_source = self
                        .discover
                        .pages
                        .sources
                        .iter()
                        .any(|s| s.name == name)
                        .then_some(name);
                } else {
                    self.discover.selected_source = self
                        .discover
                        .sources
                        .iter()
                        .any(|s| s.name() == name)
                        .then_some(name);
                }
            }
            Action::DiscoverWebsite(url) => {
                self.clear_focus();
                if self.discover.pages_mode {
                    self.discover.pages.selected_page = self
                        .discover
                        .pages
                        .hits
                        .iter()
                        .any(|h| h.hit.page.address.as_deref() == Some(&url))
                        .then_some(url);
                } else {
                    self.discover.selected_website = self
                        .discover
                        .listings
                        .iter()
                        .any(|h| h.entry.address().url() == url)
                        .then_some(url);
                }
            }
            Action::DiscoverRefresh => {
                self.clear_focus();
                if self.discover.pages_mode {
                    if self.settings.local_only {
                        self.discover.pages.error = Some("Search refresh is blocked by local-only mode. Change it in Settings to allow an explicit refresh.".into());
                    } else {
                        self.discover.pages.refresh_selected(
                            self.loader.search_fetcher(),
                            self.loader.relay_enabled(),
                        );
                    }
                } else {
                    let remote = self.discover.sources.iter().any(|s| {
                        Some(s.name()) == self.discover.selected_source.as_deref()
                            && s.remote_address().is_some()
                    });
                    if remote && self.settings.local_only {
                        self.discover.error = Some("Directory refresh is blocked by local-only mode. Change it in Settings to allow an explicit refresh.".into());
                    } else {
                        self.discover.refresh_selected(
                            self.loader.directory_fetcher(),
                            self.loader.relay_enabled(),
                        );
                    }
                }
            }
            Action::DiscoverRemove => {
                self.clear_focus();
                if self.discover.pages_mode {
                    self.discover.pages.remove_selected();
                } else {
                    self.discover.remove_selected();
                }
            }
            Action::DiscoverOpen => {
                let selected = if self.discover.pages_mode {
                    self.discover.pages.selected_page.clone()
                } else {
                    self.discover.selected_website.clone()
                };
                if let Some(url) = selected {
                    self.record("navigation-start");
                    let _ = self.browser.navigate(&url, &self.loader);
                    self.discover.open = false;
                    self.clear_focus();
                }
            }
            Action::DiscoverPage(next) => {
                self.clear_focus();
                let size = chrome::discover_page_size(self.size().1, self.discover.directories);
                if self.discover.pages_mode {
                    self.discover
                        .pages
                        .change_page(next, size, self.discover.directories);
                } else {
                    self.discover.change_page(next, size);
                }
            }
            Action::Follow(target) => {
                self.activate_link(&target);
                self.clear_focus();
            }
            Action::Find => {
                self.discover.open = false;
                self.clear_focus();
                self.find_open = true;
                self.find_edit = true;
                self.editor.activate(&self.find.query);
            }
            Action::FindNext(backwards) => {
                self.find.step(backwards);
                self.scroll_to_match();
            }
            Action::CloseFind => {
                self.find_open = false;
                self.clear_focus();
            }
            Action::Settings => {
                self.discover.open = false;
                self.settings_open = !self.settings_open;
                self.clear_focus();
            }
            Action::FinishSetup => {
                self.settings.onboarded = true;
                if self.save_settings() {
                    self.settings_open = false;
                } else {
                    self.settings.onboarded = false;
                }
                self.clear_focus();
            }
            Action::ToggleLocal => {
                self.settings.local_only = !self.settings.local_only;
                self.apply_local_only(self.settings.local_only);
                self.save_settings();
            }
            Action::ResetSettings => {
                if let Some(library) = &mut self.browser.library {
                    if let Err(error) = library.reset_settings() {
                        self.browser.library_error = Some(error.to_string());
                    }
                }
                self.settings = yami_browser::library::Settings::default();
                self.apply_local_only(true);
                self.loader.set_relay_enabled(false);
                self.settings_open = true;
            }
            Action::Library => {
                self.discover.open = false;
                self.library_open = !self.library_open;
                self.discovery.open = false;
                self.clear_focus();
            }
            Action::SaveBookmark => {
                self.discover.open = false;
                let title: String = self
                    .browser
                    .active()
                    .title()
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(40)
                    .collect();
                self.clear_focus();
                self.bookmark_edit = Some((None, false));
                self.editor.activate(&title);
                self.library_open = true;
            }
            Action::EditBookmark(index, folder) => {
                let value = self
                    .browser
                    .library
                    .as_ref()
                    .and_then(|l| l.bookmarks().get(index))
                    .map(|b| {
                        if folder {
                            b.folder.clone()
                        } else {
                            b.title.clone()
                        }
                    });
                if let Some(value) = value {
                    self.clear_focus();
                    self.bookmark_edit = Some((Some(index), folder));
                    self.editor.activate(&value);
                }
            }
            Action::OpenBookmark(index) | Action::OpenVisit(index) => {
                let url = self.browser.library.as_ref().and_then(|l| {
                    if matches!(action, Action::OpenBookmark(_)) {
                        l.bookmarks().get(index).map(|b| b.address.clone())
                    } else {
                        l.visits().get(index).map(|v| v.address.clone())
                    }
                });
                if let Some(url) = url {
                    let _ = self.browser.navigate(&url, &self.loader);
                    self.library_open = false;
                    self.clear_focus();
                }
            }
            Action::RemoveBookmark(index) => {
                if let Some(l) = &mut self.browser.library {
                    if let Some(b) = l.bookmarks().get(index).cloned() {
                        if let Err(e) = l.remove_bookmark(&b.address) {
                            self.browser.library_error = Some(e.to_string());
                        }
                    }
                }
            }
            Action::SetHomepage | Action::ToggleHistory | Action::ClearHistory => {
                let url = self.browser.active().location().address();
                if let Some(l) = &mut self.browser.library {
                    let r = match action {
                        Action::SetHomepage => l.set_homepage(&url),
                        Action::ToggleHistory => l.set_history(!l.history_enabled()),
                        _ => l.clear_history(),
                    };
                    if let Err(e) = r {
                        self.browser.library_error = Some(e.to_string());
                    }
                }
            }
            Action::LibraryPage(next) => {
                self.library_page = if next {
                    self.library_page.saturating_add(1)
                } else {
                    self.library_page.saturating_sub(1)
                };
            }
            Action::Bootstrap => {
                self.clear_focus();
                self.bootstrap_edit = true;
                self.editor.activate("Paste full node-ID@IPv4:port");
            }
            Action::ToggleRelay => {
                self.loader.set_relay_enabled(!self.loader.relay_enabled());
                self.settings.relays = self.loader.relay_enabled();
                self.save_settings();
                self.clear_focus();
            }
            Action::FindLocal => {
                self.discover.open = false;
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
                    self.library_open = false;
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
                self.clear_focus();
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
                self.record("tab-closed");
                self.browser.close_tab_with_loader(index, &self.loader);
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
            Action::Reload if matches!(self.browser.active().state, PageState::Game(_)) => {
                self.act(Action::GameReset);
            }
            Action::Reload => {
                self.browser.reload(&self.loader);
                self.clear_focus();
            }
            Action::Home => {
                self.browser.home(&self.loader);
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
    fn apply_local_only(&mut self, enabled: bool) {
        if enabled {
            self.discover.pages.cancel();
        }
        self.loader.set_local_only(enabled);
        if enabled {
            for tab in &mut self.browser.tabs {
                if matches!(tab.state, PageState::Loading) {
                    tab.state = PageState::Error("Loading cancelled by local-only mode.".into());
                }
            }
        }
    }
    fn save_settings(&mut self) -> bool {
        let result = self
            .browser
            .library
            .as_mut()
            .ok_or_else(|| {
                std::io::Error::other("Settings unavailable: browser library could not be opened")
            })
            .and_then(|library| library.set_settings(self.settings));
        match result {
            Ok(()) => {
                self.record("settings-saved");
                true
            }
            Err(error) => {
                self.record("settings-failed");
                self.browser.library_error = Some(error.to_string());
                false
            }
        }
    }
    fn scroll_to_match(&mut self) {
        if let Some(rect) = self.find.selected().and_then(|m| m.rects.first()) {
            let (_, _, scale) = self.size();
            let target = f64::from(rect.y / scale - 70.0).clamp(0.0, self.scroll_limit());
            self.browser.active_mut().target_scroll = target;
        }
    }
    fn update_find(&mut self) {
        if !self.find_open {
            return;
        }
        let query_changed = self.find_edit && self.find.query != self.editor.value;
        if query_changed {
            self.find.query = self.editor.value.clone();
        }
        let stamp = self
            .native
            .as_ref()
            .map(|c| (c.id, c.generation, c.revision, c.width, c.scale));
        if query_changed || stamp != self.find_stamp {
            self.find.matches = self
                .current_layout()
                .map_or_else(Vec::new, |doc| doc.find(&self.find.query));
            self.find.current = 0;
            self.find_stamp = stamp;
            self.scroll_to_match();
        }
    }
    fn current_layout(&self) -> Option<&NativeDocument> {
        let tab = self.browser.active();
        let PageState::Ready(page) = &tab.state else {
            return None;
        };
        let (_, _, scale) = self.size();
        let width = (self.viewport().w * scale).round() as u32;
        self.native
            .as_ref()
            .filter(|cache| {
                cache.revision == page.revision
                    && cache.id == tab.id
                    && cache.generation == tab.generation
                    && cache.width == width
                    && cache.scale == (scale as f64).to_bits()
            })
            .and_then(|cache| cache.document.as_ref().ok())
    }
    fn scroll_limit(&mut self) -> f64 {
        let (width, height, scale) = self.size();
        let error = match &self.browser.active().state {
            PageState::Error(message) => Some(message.as_str()),
            PageState::Ready(_) => self
                .native
                .as_ref()
                .and_then(|cache| cache.document.as_ref().err())
                .map(String::as_str),
            _ => None,
        };
        if let Some(message) = error {
            return chrome::error_scroll_limit(
                &mut self.text,
                width,
                height,
                scale,
                &self.browser.active().address,
                message,
            );
        }
        let height = self.viewport().h;
        self.current_layout()
            .map_or(0.0, |doc| (f64::from(doc.height / scale - height)).max(0.0))
    }
    fn scroll_to_mouse(&mut self) {
        let area = if matches!(self.browser.active().state, PageState::Error(_))
            || self
                .native
                .as_ref()
                .is_some_and(|cache| cache.document.is_err())
        {
            let (width, height, _) = self.size();
            let r = chrome::error_details_rect(width, height);
            Rect::new(r.x, r.y + 38.0, r.w, r.h - 50.0)
        } else {
            self.viewport()
        };
        let limit = self.scroll_limit();
        let fraction = ((self.mouse.1 - area.y) / area.h).clamp(0.0, 1.0);
        let tab = self.browser.active_mut();
        tab.target_scroll = f64::from(fraction) * limit;
        tab.scroll = tab.target_scroll;
    }
    fn link(&self) -> Option<String> {
        let (width, height, _) = self.size();
        if self.discover.open {
            return None;
        }
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
        let paint_start = Instant::now();
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
        let tab = self.browser.active();
        self.game_presentation.record(tab.id, matches!(&tab.state, PageState::Game(g) if g.snapshot.status == yami_games::Status::Running), if let PageState::Game(g)=&tab.state {g.snapshot.frames}else{0}, paint_start.elapsed());
        self.frames += 1;
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
                cache.revision == page.revision
                    && cache.id == tab.id
                    && cache.generation == tab.generation
                    && cache.width == doc_width
                    && cache.scale == (scale as f64).to_bits()
            });
            if !valid {
                self.native = Some(CachedDocument {
                    revision: page.revision,
                    id: tab.id,
                    generation: tab.generation,
                    width: doc_width,
                    scale: (scale as f64).to_bits(),
                    document: NativeDocument::layout_with_assets(
                        &page.document,
                        &page.style,
                        doc_width as f32,
                        scale,
                        &mut self.text,
                        &page.images,
                    )
                    .map_err(|e| e.to_string()),
                });
            }
        } else {
            // Do not retain a closed tab's decoded images/shaped document.
            self.native = None;
        }
        self.update_find();
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
        let value = if self.editor.focus.is_some()
            && !self.find_edit
            && self.discover_edit.is_none()
            && !self.game_open
        {
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
            editing: self.editor.focus.is_some()
                && !self.find_edit
                && self.discover_edit.is_none()
                && !self.game_open,
            value,
            selection: self.editor.selected,
            selection_range: self.editor.range(),
            caret: self.editor.caret,
            focus: focus.as_ref(),
        };
        let viewport = self.viewport();
        let mut c = Canvas::new(pix, &mut self.text, scale);
        let immersive = self.game_immersive
            && matches!(self.browser.active().state, PageState::Game(_))
            && !self.settings_open
            && !self.library_open
            && !self.discover.open
            && !self.discovery.open
            && !self.find_open
            && !self.game_open;
        let mut scene = if immersive {
            Scene::default()
        } else {
            let mut scene = chrome::paint(&mut c, &self.browser, &ui);
            chrome::relay_setting(&mut c, &mut scene, &ui, self.loader.relay_enabled());
            scene
        };
        match &self.browser.active().state {
            PageState::Game(game) => super::game_view::paint(
                &mut c,
                &mut scene,
                &ui,
                &game.snapshot,
                &mut self.game_presentation,
                immersive,
            ),
            PageState::Home => chrome::home(&mut c, &mut scene, &ui, &self.browser),
            PageState::Loading => {
                chrome::document_viewport(&mut c, &mut scene, &ui);
                let verifying = matches!(
                    self.browser.active().location(),
                    Location::Ynps(_) | Location::Yami(_) | Location::Named(_)
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
                chrome::error_page(&mut c, &mut scene, &ui, &self.browser, message)
            }
            PageState::Ready(_) => {
                let tab = self.browser.active();
                // The YLang document paints its own paper edge to edge in the
                // viewport, so layout width, drawing and link hit-tests agree.
                match self.native.as_ref().map(|cache| &cache.document) {
                    Some(Ok(doc)) => {
                        doc.draw_interactive(
                            c.pix,
                            viewport.x * scale,
                            viewport.y * scale,
                            tab.scroll as f32 * scale,
                            viewport.h * scale,
                            c.text,
                            if !self.library_open
                                && !self.discovery.open
                                && viewport.contains(self.mouse.0, self.mouse.1)
                            {
                                Some((
                                    (self.mouse.0 - viewport.x) * scale,
                                    (self.mouse.1 - viewport.y + tab.scroll as f32) * scale,
                                ))
                            } else {
                                None
                            },
                            self.pressed_link.is_some(),
                        );
                        for (target, rect) in doc.links() {
                            let top =
                                (viewport.y + rect.y / scale - tab.scroll as f32).max(viewport.y);
                            let bottom = (viewport.y + (rect.y + rect.height) / scale
                                - tab.scroll as f32)
                                .min(viewport.y + viewport.h);
                            if bottom > top {
                                scene.hits.push(chrome::Hit {
                                    rect: Rect::new(
                                        viewport.x + rect.x / scale,
                                        top,
                                        rect.width / scale,
                                        bottom - top,
                                    ),
                                    action: Action::Follow(target.into()),
                                });
                            }
                        }
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
                        chrome::error_page(&mut c, &mut scene, &ui, &self.browser, error)
                    }
                    None => chrome::document_viewport(&mut c, &mut scene, &ui),
                }
                chrome::round_viewport(&mut c, &ui);
            }
        }
        chrome::discovery_panel(&mut c, &mut scene, &ui, &self.discovery);
        if self.library_open {
            chrome::library_panel(&mut c, &mut scene, &ui, &self.browser, self.library_page);
        }
        chrome::discover_panel(
            &mut c,
            &mut scene,
            &ui,
            &self.discover,
            self.discover_edit
                .map(|field| (field, self.editor.value.as_str())),
            self.settings.local_only,
        );
        if self.settings_open {
            chrome::settings_panel(
                &mut c,
                &mut scene,
                &ui,
                self.settings,
                self.browser.library_error.as_deref(),
            );
        }
        if self.find_open && !self.settings_open {
            if let Some(selected) = self.find.selected() {
                for rect in &selected.rects {
                    let top = (viewport.y + rect.y / scale - self.browser.active().scroll as f32)
                        .max(viewport.y);
                    let bottom = (viewport.y + (rect.y + rect.height) / scale
                        - self.browser.active().scroll as f32)
                        .min(viewport.y + viewport.h);
                    if bottom > top {
                        c.round(
                            Rect::new(
                                viewport.x + rect.x / scale,
                                top,
                                rect.width / scale,
                                bottom - top,
                            ),
                            1.0,
                            [255, 190, 25, 110],
                            [255, 190, 25, 110],
                            Some([255, 190, 25, 255]),
                        );
                    }
                }
            }
            chrome::find_bar(&mut c, &mut scene, &ui, &self.find, self.find_edit);
        }
        if let Some(hit) = ui
            .focus
            .and_then(|action| scene.hits.iter().find(|hit| &hit.action == action))
        {
            let r = hit.rect;
            c.line(
                &[
                    (r.x, r.y),
                    (r.x + r.w, r.y),
                    (r.x + r.w, r.y + r.h),
                    (r.x, r.y + r.h),
                    (r.x, r.y),
                ],
                [255, 255, 255, 255],
                2.0,
            );
        }
        if self.game_open {
            super::game_view::launcher(
                &mut c,
                &mut scene,
                &ui,
                &self.editor.value,
                self.game_error.as_deref(),
            );
        }
        self.scene = scene;
    }
    fn activate_link(&mut self, target: &str) {
        if let Some(key) = target.strip_prefix("game:") {
            match self.browser.play_hosted(key, Arc::clone(&self.game_wake)) {
                Ok(()) => {
                    self.game_immersive = true;
                    self.clear_focus();
                }
                Err(error) => {
                    self.browser.active_mut().state = PageState::Error(error);
                }
            }
        } else {
            let _ = self.browser.follow(target, &self.loader);
        }
    }
    /// Pointer coordinates use the same logical surface as painting, after DPI conversion.
    fn game_pointer(&self, button: Option<(MouseButton, bool)>) -> bool {
        let PageState::Game(game) = &self.browser.active().state else {
            return false;
        };
        let bit = button.map(|(b, _)| {
            if b == MouseButton::Left {
                yami_games::MOUSE_LEFT
            } else {
                yami_games::MOUSE_RIGHT
            }
        });
        // Releases reach the owning session even outside its surface or under a modal.
        if let (Some(bit), Some((_, false))) = (bit, button) {
            game.session.key(bit, false);
        }
        let (width, height, _) = self.size();
        let area = if self.game_immersive {
            Rect::new(0., 0., width, height)
        } else {
            chrome::viewport(width, height)
        };
        let view = super::game_view::surface(area);
        let owns = self.editor.focus.is_none()
            && self.keyboard_focus.is_none()
            && !self.game_open
            && !self.settings_open
            && !self.library_open
            && !self.discover.open
            && !self.discovery.open
            && !self.find_open
            && !self.modifiers.control_key()
            && !self.modifiers.super_key()
            && !self.modifiers.alt_key()
            && game.snapshot.status == yami_games::Status::Running
            && view.contains(self.mouse.0, self.mouse.1);
        if owns {
            let x = ((self.mouse.0 - view.x) * yami_games::WIDTH as f32 / view.w).floor() as i32;
            let y = ((self.mouse.1 - view.y) * yami_games::HEIGHT as f32 / view.h).floor() as i32;
            game.session.pointer(x, y);
            if let (Some(bit), Some((_, true))) = (bit, button) {
                game.session.key(bit, true);
            }
        } else {
            game.session.pointer(-1, -1);
        }
        owns
    }
    fn game_key(&mut self, key: &winit::keyboard::PhysicalKey, down: bool, repeat: bool) -> bool {
        use winit::keyboard::{KeyCode, PhysicalKey};
        fn key_bit(code: &KeyCode) -> u32 {
            match code {
                KeyCode::ArrowLeft | KeyCode::KeyA => yami_games::LEFT,
                KeyCode::ArrowRight | KeyCode::KeyD => yami_games::RIGHT,
                KeyCode::Space | KeyCode::Enter => yami_games::START,
                KeyCode::KeyR => yami_games::RESET,
                KeyCode::ArrowUp | KeyCode::KeyW => yami_games::UP,
                KeyCode::ArrowDown | KeyCode::KeyS => yami_games::DOWN,
                KeyCode::KeyZ | KeyCode::KeyJ => yami_games::ACTION,
                KeyCode::KeyX | KeyCode::KeyK => yami_games::SECONDARY,
                _ => 0,
            }
        }
        let bit = if let PhysicalKey::Code(code) = key {
            key_bit(code)
        } else {
            0
        };
        let PageState::Game(game) = &self.browser.active().state else {
            return false;
        };
        // Releases are always forwarded, including while an editor/modal owns focus.
        if !down && bit != 0 {
            if let PhysicalKey::Code(code) = key {
                self.game_keys.remove(code);
            }
            let still_held = self.game_keys.iter().any(|code| key_bit(code) == bit);
            if !still_held {
                game.session.key(bit, false);
            }
        }
        if matches!(key, PhysicalKey::Code(KeyCode::Tab)) && down && !repeat {
            self.game_keys.clear();
            game.session.clear_input();
            game.session.pause();
        }
        if self.modifiers.control_key()
            || self.modifiers.super_key()
            || self.modifiers.alt_key()
            || self.editor.focus.is_some()
            || self.keyboard_focus.is_some()
            || self.game_open
            || self.settings_open
            || self.library_open
            || self.discover.open
            || self.discovery.open
            || self.find_open
        {
            return false;
        }
        if matches!(key, PhysicalKey::Code(KeyCode::F3)) {
            if down && !repeat {
                self.game_presentation.diagnostics = !self.game_presentation.diagnostics;
                self.redraw();
            }
            return true;
        }
        if matches!(key, PhysicalKey::Code(KeyCode::KeyP | KeyCode::Escape)) {
            if down && !repeat {
                self.act(Action::GamePause);
                self.redraw();
            }
            return true;
        }
        if bit != 0 {
            if down && !repeat {
                if let PhysicalKey::Code(code) = key {
                    self.game_keys.insert(*code);
                }
                game.session.key(bit, true);
            }
            return true;
        }
        false
    }
    fn keyboard(&mut self, key: &Key, text: Option<&str>, event_loop: Option<&ActiveEventLoop>) {
        // Windows exposes AltGr as Ctrl+Alt. Preserve printable input from
        // European keyboard layouts (including IPv6 brackets) in the editor.
        let altgr = self.modifiers.control_key() && self.modifiers.alt_key();
        let command = (self.modifiers.control_key() || self.modifiers.super_key()) && !altgr;
        if command {
            if let Key::Character(ch) = key {
                match ch.to_lowercase().as_str() {
                    "f" => {
                        self.act(Action::Find);
                        return;
                    }
                    "g" => {
                        self.act(Action::FindNext(self.modifiers.shift_key()));
                        return;
                    }
                    "c" if self.modifiers.shift_key() && self.editor.focus.is_none() => {
                        if self.clipboard.is_none() {
                            self.clipboard = arboard::Clipboard::new().ok();
                        }
                        if let Some(clipboard) = &mut self.clipboard {
                            let _ = clipboard.set_text(self.browser.active().location().address());
                        }
                        return;
                    }
                    "d" => {
                        self.act(Action::SaveBookmark);
                        return;
                    }
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
                        self.editor.anchor = None;
                        self.editor.caret = self.editor.value.len();
                        self.redraw();
                        return;
                    }
                    "c" | "x" if self.editor.focus.is_some() => {
                        if self.editor.selected {
                            if self.clipboard.is_none() {
                                self.clipboard = arboard::Clipboard::new().ok();
                            }
                            if let Some(clipboard) = &mut self.clipboard {
                                let _ = clipboard.set_text(
                                    self.editor.value[self.editor.range().unwrap()].to_string(),
                                );
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
                Key::Named(NamedKey::Home) => {
                    self.act(Action::Home);
                    return;
                }
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
                self.game_open = false;
                self.discover.pages.cancel();
                self.discover.open = false;
                self.discovery.open = false;
                self.library_open = false;
                self.settings_open = false;
                self.find_open = false;
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
                if let Some(field) = self.discover_edit {
                    let index = self
                        .scene
                        .hits
                        .iter()
                        .position(|hit| hit.action == Action::DiscoverField(field));
                    self.clear_focus();
                    self.keyboard_focus = index;
                } else {
                    self.editor.focus = None;
                }
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
                } else if self.discover.open
                    && self.discover.pages_mode
                    && !self.discover.directories
                    && self.discover.pages.selected_page.is_some()
                    && self.keyboard_focus.is_none()
                {
                    self.act(Action::DiscoverOpen);
                } else if let Some(hit) = self.keyboard_focus.and_then(|i| self.scene.hits.get(i)) {
                    self.act(hit.action.clone());
                }
            }
            Key::Named(NamedKey::Backspace) if self.editor.focus.is_some() => {
                self.editor.backspace()
            }
            Key::Named(NamedKey::Delete) if self.editor.focus.is_some() => self.editor.delete(),
            Key::Named(NamedKey::ArrowLeft) if self.editor.focus.is_some() => {
                let to = if self.editor.selected && !self.modifiers.shift_key() {
                    self.editor.range().unwrap().start
                } else {
                    self.editor.previous()
                };
                self.editor.move_caret(to, self.modifiers.shift_key());
            }
            Key::Named(NamedKey::ArrowRight) if self.editor.focus.is_some() => {
                let to = if self.editor.selected && !self.modifiers.shift_key() {
                    self.editor.range().unwrap().end
                } else {
                    self.editor.next()
                };
                self.editor.move_caret(to, self.modifiers.shift_key());
            }
            Key::Named(NamedKey::Home) if self.editor.focus.is_some() => {
                self.editor.move_caret(0, self.modifiers.shift_key());
            }
            Key::Named(NamedKey::End) if self.editor.focus.is_some() => {
                self.editor
                    .move_caret(self.editor.value.len(), self.modifiers.shift_key());
            }
            Key::Named(NamedKey::ArrowDown) | Key::Named(NamedKey::ArrowUp)
                if self.discover.open
                    && self.discover.pages_mode
                    && !self.discover.directories
                    && self.editor.focus.is_none() =>
            {
                let next = key == &Key::Named(NamedKey::ArrowDown);
                let size = chrome::discover_page_size(self.size().1, false);
                self.discover.pages.move_selection(next, size);
                self.keyboard_focus = None;
            }
            Key::Named(NamedKey::PageDown) | Key::Named(NamedKey::ArrowDown)
                if self.discover.open && self.editor.focus.is_none() =>
            {
                self.act(Action::DiscoverPage(true));
            }
            Key::Named(NamedKey::PageUp) | Key::Named(NamedKey::ArrowUp)
                if self.discover.open && self.editor.focus.is_none() =>
            {
                self.act(Action::DiscoverPage(false));
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
            Key::Named(NamedKey::F3) => self.act(Action::FindNext(self.modifiers.shift_key())),
            Key::Named(NamedKey::F4) if self.modifiers.alt_key() => {
                if let Some(event_loop) = event_loop {
                    event_loop.exit();
                }
            }
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
    fn exiting(&mut self, _: &ActiveEventLoop) {
        for tab in &self.browser.tabs {
            if let PageState::Game(game) = &tab.state {
                game.session.stop();
            }
        }
        self.record("shutdown");
    }
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        let directories_changed = self.discover.poll();
        let discovery_changed = self.discovery.poll();
        let games_changed = self.browser.poll_games();
        if games_changed {
            self.redraw();
        }
        if self.browser.poll(&self.loader) || discovery_changed || directories_changed {
            self.record(
                if matches!(self.browser.active().state, PageState::Error(_)) {
                    "navigation-failed"
                } else {
                    "navigation-ready"
                },
            );
            self.redraw();
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.record("window-resized");
                self.redraw();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.render() {
                    self.fatal = Some(error.to_string());
                    event_loop.exit();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                if self.game_key(
                    &event.physical_key,
                    event.state == ElementState::Pressed,
                    event.repeat,
                ) {
                    return;
                }
                if event.state == ElementState::Pressed && !self.composing {
                    self.keyboard(&event.logical_key, event.text.as_deref(), Some(event_loop));
                }
            }
            WindowEvent::Ime(Ime::Disabled) => {
                self.composing = false;
            }
            WindowEvent::Occluded(true) => {
                self.game_keys.clear();
                self.browser.pause_active_game();
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
                let previous_hit = self.scene.hit(self.mouse.0, self.mouse.1);
                let previous_link = self.link();
                let scale = self.window.as_ref().unwrap().scale_factor() as f32;
                self.mouse = (position.x as f32 / scale, position.y as f32 / scale);
                self.game_pointer(None);
                if self.scroll_drag {
                    self.scroll_to_mouse();
                }
                let cursor = match self.scene.hit(self.mouse.0, self.mouse.1) {
                    Some(Action::Address | Action::DiscoverField(_)) => CursorIcon::Text,
                    Some(_) => CursorIcon::Pointer,
                    None if self.link().is_some() => CursorIcon::Pointer,
                    _ => CursorIcon::Default,
                };
                self.window.as_ref().unwrap().set_cursor(cursor);
                if self.scroll_drag
                    || previous_hit != self.scene.hit(self.mouse.0, self.mouse.1)
                    || previous_link != self.link()
                {
                    self.redraw();
                }
            }
            WindowEvent::CursorLeft { .. } => {
                self.mouse = (-1.0, -1.0);
                self.game_pointer(None);
                if let PageState::Game(game) = &self.browser.active().state {
                    game.session
                        .key(yami_games::MOUSE_LEFT | yami_games::MOUSE_RIGHT, false);
                }
                self.redraw();
            }
            WindowEvent::MouseInput { state, button, .. }
                if matches!(button, MouseButton::Left | MouseButton::Right)
                    && self.game_pointer(Some((button, state == ElementState::Pressed))) => {}
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(action) = self.scene.hit(self.mouse.0, self.mouse.1) {
                    self.act(action);
                } else if let Some(target) = self.link() {
                    self.pressed_link = Some(target);
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
            } => {
                self.scroll_drag = false;
                if let Some(target) = self.pressed_link.take() {
                    if self.scene.hit(self.mouse.0, self.mouse.1).is_none()
                        && self.link().as_ref() == Some(&target)
                    {
                        self.activate_link(&target);
                    }
                }
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => f64::from(y) * 54.0,
                    MouseScrollDelta::PixelDelta(p) => {
                        p.y / self.window.as_ref().unwrap().scale_factor()
                    }
                };
                if self.discover.open {
                    let size = chrome::discover_page_size(self.size().1, self.discover.directories);
                    self.discover.change_page(amount < 0.0, size);
                } else {
                    let limit = self.scroll_limit();
                    let tab = self.browser.active_mut();
                    tab.target_scroll = (tab.target_scroll - amount).clamp(0.0, limit);
                }
                self.redraw();
            }
            WindowEvent::Focused(false) => {
                self.game_keys.clear();
                self.browser.pause_active_game();
                self.modifiers = ModifiersState::empty();
                self.scroll_drag = false;
                self.composing = false;
            }
            _ => {}
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(mut smoke) = self.smoke.take() {
            match smoke.tick(self, event_loop) {
                Ok(false) => self.smoke = Some(smoke),
                Ok(true) => {}
                Err(error) => {
                    let _ = smoke.write("FAIL", self.frames);
                    self.fatal = Some(error);
                    event_loop.exit();
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(30),
            ));
            return;
        }
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
    fn game_path() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../games/breakout/game.yami-game.json")
    }
    fn game_wait(app: &mut Desktop, check: impl Fn(&yami_games::Snapshot) -> bool) {
        let start = Instant::now();
        loop {
            app.browser.poll_games();
            let PageState::Game(game) = &app.browser.active().state else {
                panic!("game tab expected");
            };
            if check(&game.snapshot) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{:?}",
                game.snapshot
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn local_game_launcher_keyboard_modals_pause_tabs_and_document_isolation() {
        use winit::keyboard::{KeyCode, PhysicalKey};
        let mut app = headless(1440., 900.);
        app.settings_open = false;
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        let document_id = app.browser.active().id;
        app.act(Action::OpenGame);
        app.editor.value = game_path().to_string_lossy().into();
        app.commit();
        assert_eq!(app.browser.tabs.len(), 2);
        assert!(matches!(app.browser.tabs[0].state, PageState::Ready(_)));
        assert_eq!(app.browser.tabs[0].id, document_id);
        game_wait(&mut app, |s| {
            s.frames > 0 && s.status == yami_games::Status::Running
        });
        let _ = frame(&mut app);
        assert!(app.game_immersive);
        assert!(!app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::Address));
        // Tab transfers keyboard ownership to the toolbar and releases movement.
        app.game_key(&PhysicalKey::Code(KeyCode::KeyA), true, false);
        app.game_key(&PhysicalKey::Code(KeyCode::Tab), true, false);
        app.keyboard(&Key::Named(NamedKey::Tab), None, None);
        game_wait(&mut app, |s| s.status == yami_games::Status::Paused);
        assert!(app.game_keys.is_empty());
        assert!(!app.game_key(&PhysicalKey::Code(KeyCode::Enter), true, false));
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        game_wait(&mut app, |s| s.status == yami_games::Status::Running);

        app.act(Action::GameLayout);
        game_wait(&mut app, |s| s.status == yami_games::Status::Paused);
        let _ = frame(&mut app);
        assert!(!app.game_immersive);
        assert!(app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::Address));
        app.act(Action::GameLayout);
        app.act(Action::GamePause);
        game_wait(&mut app, |s| s.status == yami_games::Status::Running);
        let _ = frame(&mut app);
        for action in [
            Action::GamePause,
            Action::GameReset,
            Action::GameStop,
            Action::GameLayout,
        ] {
            assert!(app.scene.hits.iter().any(|hit| hit.action == action));
        }
        assert!(app.game_key(&PhysicalKey::Code(KeyCode::Space), true, false));
        assert!(app.game_key(&PhysicalKey::Code(KeyCode::Space), false, false));
        game_wait(&mut app, |s| {
            s.frame.as_ref().is_some_and(|f| {
                f.commands.iter().any(
                    |c| matches!(c,yami_games::Command::Text{text,..} if text=="SPACE to launch"),
                )
            })
        });
        app.game_key(&PhysicalKey::Code(KeyCode::KeyA), true, false);
        app.game_key(&PhysicalKey::Code(KeyCode::ArrowLeft), true, false);
        app.game_key(&PhysicalKey::Code(KeyCode::KeyA), false, false);
        assert!(app.game_keys.contains(&KeyCode::ArrowLeft));
        app.act(Action::Address);
        assert!(app.game_keys.is_empty());
        game_wait(&mut app, |s| s.status == yami_games::Status::Paused);
        assert!(!app.game_key(&PhysicalKey::Code(KeyCode::KeyA), true, false));
        app.clear_focus();
        app.act(Action::GamePause);
        game_wait(&mut app, |s| s.status == yami_games::Status::Running);
        app.act(Action::Tab(0));
        assert!(matches!(app.browser.active().state, PageState::Ready(_)));
        app.act(Action::Tab(1));
        game_wait(&mut app, |s| s.status == yami_games::Status::Paused);
        app.act(Action::GameStop);
        game_wait(&mut app, |s| s.status == yami_games::Status::Stopped);
        app.act(Action::GameReset);
        game_wait(&mut app, |s| {
            s.frames > 0 && s.status == yami_games::Status::Running
        });
        let _ = frame(&mut app);
        app.act(Action::Close(1));
        let _ = frame(&mut app);
        assert_eq!(app.browser.tabs.len(), 1);
        assert!(matches!(app.browser.active().state, PageState::Ready(_)));
        // Executable manifests can never be opened by document addresses or links.
        assert!(app
            .browser
            .navigate(&game_path().to_string_lossy(), &app.loader)
            .is_err());
    }
    #[test]
    fn game_rendering_preserves_aspect_and_clips_at_multiple_sizes_and_dpi() {
        let png = yami_render::assets::encode_png(2, 1, &[255, 0, 0, 128, 0, 255, 0, 255]).unwrap();
        let mut textures = yami_games::Assets::default();
        textures
            .insert(
                &yami_games::AssetResource {
                    id: 0,
                    key: "test".into(),
                    kind: yami_games::AssetKind::Png,
                    bytes: png.len(),
                    sha256: yami_identity::sha256(&png)
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect(),
                },
                &png,
            )
            .unwrap();
        let textures = Arc::new(textures);
        let mut app = headless(900., 600.);
        app.settings_open = false;
        app.browser
            .open_game(game_path(), Arc::clone(&app.game_wake))
            .unwrap();
        game_wait(&mut app, |s| s.frames > 1);
        if let PageState::Game(game) = &app.browser.active().state {
            game.session.pause();
        }
        for (w, h, scale, immersive) in [
            (900., 600., 1., true),
            (1440., 900., 1.25, true),
            (1200., 800., 2., true),
            (900., 600., 1.5, false),
        ] {
            app.headless = Some((w, h, scale));
            app.game_immersive = immersive;
            let original = if let PageState::Game(game) = &app.browser.active().state {
                Arc::clone(game.snapshot.frame.as_ref().unwrap())
            } else {
                panic!()
            };
            let mut before = Pixmap::new((w * scale) as u32, (h * scale) as u32).unwrap();
            app.compose(&mut before);
            let PageState::Game(game) = &mut app.browser.active_mut().state else {
                panic!()
            };
            game.snapshot.frame = Some(Arc::new(yami_games::Frame {
                assets: textures.clone(),
                commands: vec![
                    yami_games::Command::Clear(0xff0000ff),
                    yami_games::Command::Rect {
                        x: 320,
                        y: 240,
                        w: 24,
                        h: 12,
                        color: 0x00ff00ff,
                    },
                    yami_games::Command::Rect {
                        x: 380,
                        y: 240,
                        w: 24,
                        h: 12,
                        color: 0x0000ffff,
                    },
                    yami_games::Command::Sprite {
                        asset: 0,
                        sx: 0,
                        sy: 0,
                        sw: 1,
                        sh: 1,
                        x: 380,
                        y: 240,
                        w: 24,
                        h: 12,
                        layer: 1,
                    },
                    yami_games::Command::Sprite {
                        asset: 0,
                        sx: 1,
                        sy: 0,
                        sw: 1,
                        sh: 1,
                        x: -12,
                        y: 100,
                        w: 24,
                        h: 12,
                        layer: 2,
                    },
                    yami_games::Command::Text {
                        x: 639,
                        y: 479,
                        size: 48,
                        text: "OVERFLOW".into(),
                        color: 0xffffffff,
                    },
                ],
            }));
            let mut after = Pixmap::new(before.width(), before.height()).unwrap();
            app.compose(&mut after);
            let area = if immersive {
                Rect::new(0., 0., w, h)
            } else {
                chrome::viewport(w, h)
            };
            let view = super::super::game_view::surface(area);
            assert!((view.w / view.h - 4. / 3.).abs() < 0.001);
            assert!(area.contains(view.x, view.y));
            assert!(view.x + view.w <= area.x + area.w);
            assert!(view.y + view.h <= area.y + area.h);
            let surface = Rect::new(
                (view.x * scale).round(),
                (view.y * scale).round(),
                (view.w * scale).floor(),
                (view.h * scale).floor(),
            );
            let green = after
                .pixel(
                    (surface.x + surface.w * 0.516) as u32,
                    (surface.y + surface.h * 0.510) as u32,
                )
                .unwrap();
            assert_eq!(
                (green.red(), green.green(), green.blue()),
                (0, 255, 0),
                "logical geometry must scale with the surface at {scale} DPI"
            );
            let blended = after
                .pixel(
                    (surface.x + surface.w * 0.61) as u32,
                    (surface.y + surface.h * 0.51) as u32,
                )
                .unwrap();
            assert_eq!(
                (blended.red(), blended.green(), blended.blue()),
                (128, 0, 127),
                "alpha sprite source must not sample the neighboring green frame"
            );
            let clipped = after
                .pixel(
                    (surface.x + surface.w * 0.008) as u32,
                    (surface.y + surface.h * 0.22) as u32,
                )
                .unwrap();
            assert_eq!(
                (clipped.red(), clipped.green(), clipped.blue()),
                (0, 255, 0),
                "source rectangles and negative destinations clip correctly"
            );
            for y in 0..after.height() {
                for x in 0..after.width() {
                    if !surface.contains(x as f32, y as f32) {
                        assert_eq!(
                            before.pixel(x, y),
                            after.pixel(x, y),
                            "guest escaped surface at {x},{y}"
                        );
                    }
                }
            }
            assert!(after
                .pixels()
                .iter()
                .any(|p| p.red() == 255 && p.green() == 0 && p.blue() == 0));
            if let PageState::Game(game) = &mut app.browser.active_mut().state {
                game.snapshot.frame = Some(original);
            }
        }
    }

    #[test]
    #[ignore = "manual software rendering benchmark"]
    fn measure_native_game_rendering() {
        let mut app = headless(1440., 900.);
        app.settings_open = false;
        app.browser
            .open_game(game_path(), Arc::clone(&app.game_wake))
            .unwrap();
        game_wait(&mut app, |s| s.frames > 1);
        if let PageState::Game(game) = &app.browser.active().state {
            game.session.pause();
        }
        let (_, bytes) = yami_games::load_local(&game_path()).unwrap();
        let mut vm = yami_games::Runtime::new(&bytes).unwrap();
        vm.frame(0, yami_games::START).unwrap();
        vm.frame(0, yami_games::START).unwrap();
        let mut pix = Pixmap::new(1440, 900).unwrap();
        let mut times = Vec::new();
        for _ in 0..180 {
            if let PageState::Game(game) = &mut app.browser.active_mut().state {
                game.snapshot.frame = Some(vm.frame(0, 0).unwrap());
            }
            let now = Instant::now();
            app.compose(&mut pix);
            times.push(now.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!("native compose 1440x900: avg={:.3}ms p50={:.3}ms p95={:.3}ms max={:.3}ms (180 gameplay frames)",times.iter().sum::<f64>()/times.len() as f64,times[90],times[171],times[179]);
    }

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

    #[test]
    fn keyboard_find_unicode_scroll_modal_escape_and_closed_layout_cleanup() {
        let mut app = headless(1440.0, 900.0);
        app.browser.navigate("local:welcome", &app.loader).unwrap();
        settle(&mut app);
        frame(&mut app);
        app.act(Action::Navigate("local:about"));
        settle(&mut app);
        app.modifiers = ModifiersState::ALT;
        app.keyboard(&Key::Named(NamedKey::ArrowLeft), None, None);
        settle(&mut app);
        assert_eq!(app.browser.active().address, "local:welcome");
        app.keyboard(&Key::Named(NamedKey::ArrowRight), None, None);
        settle(&mut app);
        assert_eq!(app.browser.active().address, "local:about");
        app.modifiers = ModifiersState::CONTROL;
        app.keyboard(&Key::Character("r".into()), None, None);
        settle(&mut app);
        app.keyboard(&Key::Named(NamedKey::F5), None, None);
        settle(&mut app);
        app.keyboard(&Key::Character("l".into()), None, None);
        assert!(app.editor.selected);
        app.keyboard(&Key::Character("t".into()), None, None);
        assert_eq!(app.browser.tabs.len(), 2);
        app.keyboard(&Key::Character("w".into()), None, None);
        assert_eq!(app.browser.tabs.len(), 1);
        if let PageState::Ready(page) = &mut app.browser.active_mut().state {
            let source = format!(
                "@page 0.2\n  title Search\n{}\ncafé [strong: unique] café\n",
                "Long paragraph for scroll.\n\n".repeat(90)
            );
            page.document = ylang::parse(&source).unwrap();
            page.revision += 1;
        }
        frame(&mut app);
        app.keyboard(&Key::Character("f".into()), None, None);
        app.editor.replace("café");
        frame(&mut app);
        assert_eq!(app.find.matches.len(), 2);
        let scroll_limit = app.scroll_limit();
        assert!(
            app.browser.active().target_scroll > 500.0,
            "target={} limit={} rect={:?}",
            app.browser.active().target_scroll,
            scroll_limit,
            app.find.selected()
        );
        app.modifiers = ModifiersState::empty();
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        assert_eq!(app.find.current, 1);
        app.modifiers = ModifiersState::SHIFT;
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        assert_eq!(app.find.current, 0);
        app.settings_open = true;
        app.library_open = true;
        app.keyboard(&Key::Named(NamedKey::Escape), None, None);
        assert!(!app.find_open && !app.settings_open && !app.library_open);
        app.act(Action::Close(0));
        frame(&mut app);
        assert!(app.native.is_none());
    }

    #[test]
    fn switching_from_find_or_bootstrap_to_address_clears_editor_mode() {
        let mut app = headless(1200.0, 800.0);
        for previous in [Action::Find, Action::Bootstrap] {
            app.act(previous);
            app.act(Action::Address);
            app.editor.replace("local:about");
            app.commit();
            settle(&mut app);
            assert_eq!(app.browser.active().address, "local:about");
            assert!(!app.find_edit && !app.bootstrap_edit);
        }
    }
    #[test]
    fn failed_first_run_save_stays_visible_and_reset_cancels_loading_state() {
        let mut app = headless(1200.0, 800.0);
        app.settings_open = true;
        app.act(Action::FinishSetup);
        assert!(app.settings_open, "failed save must remain visible");
        app.browser.active_mut().state = PageState::Loading;
        app.act(Action::ResetSettings);
        assert!(matches!(app.browser.active().state, PageState::Error(_)));
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

    fn click_action(app: &mut Desktop, action: Action, in_page: bool) {
        frame(app);
        let hit = app
            .scene
            .hits
            .iter()
            .find(|hit| hit.action == action && (!in_page || hit.rect.x >= chrome::SIDEBAR_WIDTH))
            .expect("visible control")
            .rect;
        let action = app
            .scene
            .hit(hit.x + hit.w / 2.0, hit.y + hit.h / 2.0)
            .unwrap();
        app.act(action);
    }

    #[test]
    fn start_page_actions_use_the_existing_editor_and_local_workflows() {
        let mut app = headless(900.0, 600.0);
        click_action(&mut app, Action::Address, true);
        app.editor.replace("local:welcome");
        app.commit();
        settle(&mut app);
        assert_eq!(app.browser.active().title(), "Welcome to YamiNet");
        assert!(frame(&mut app).width() > 0);
        assert!(app.current_layout().is_some());
        app.act(Action::Navigate(""));
        click_action(&mut app, Action::Discover, true);
        assert!(app.discover.open);
        app.act(Action::Navigate(""));
        click_action(&mut app, Action::OpenGame, true);
        assert!(app.game_open);
        assert_eq!(app.browser.tabs.len(), 1);
    }

    #[test]
    fn error_diagnostics_scroll_and_recovery_uses_hit_tested_controls() {
        let mut app = headless(900.0, 600.0);
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        app.act(Action::Navigate("not-an-address"));
        app.browser.active_mut().state = PageState::Error(format!(
            "{}\nFINAL DIAGNOSTIC",
            "Long café diagnostic with a full node identity. ".repeat(100)
        ));
        frame(&mut app);
        let limit = app.scroll_limit();
        assert!(limit > 0.0);
        app.browser.active_mut().target_scroll = limit;
        app.browser.active_mut().scroll = limit;
        frame(&mut app);
        assert_eq!(app.browser.active().scroll, limit);
        click_action(&mut app, Action::Back, true);
        // Invalid address input never adds a history entry. Start-page recovery
        // is always available, regardless of whether back has somewhere to go.
        app.act(Action::Navigate("not-an-address"));
        click_action(&mut app, Action::Navigate(""), true);
        assert!(matches!(app.browser.active().state, PageState::Home));
        app.act(Action::Navigate("not-an-address"));
        click_action(&mut app, Action::Reload, true);
        assert!(matches!(app.browser.active().state, PageState::Error(_)));
        assert_eq!(app.browser.active().address, "not-an-address");
    }

    #[test]
    fn crowded_sidebar_keeps_tabs_and_utilities_separate_at_minimum_size() {
        let mut app = headless(900.0, 600.0);
        for _ in 1..crate::model::MAX_TABS {
            app.browser.open_tab();
        }
        for _ in 0..crate::model::MAX_TABS {
            frame(&mut app);
            for hit in app
                .scene
                .hits
                .iter()
                .filter(|h| matches!(h.action, Action::Tab(_) | Action::Close(_)))
            {
                assert!(hit.rect.y + hit.rect.h < 600.0 - 188.0);
                assert_eq!(
                    app.scene.hit(hit.rect.x + 1.0, hit.rect.y + 1.0),
                    Some(hit.action.clone())
                );
            }
            if app.browser.active == 0 {
                break;
            }
            let earlier = app
                .scene
                .hits
                .iter()
                .find(|h| matches!(h.action, Action::Tab(i) if i < app.browser.active))
                .unwrap()
                .action
                .clone();
            app.act(earlier);
        }
        assert_eq!(app.browser.active, 0);
    }

    #[test]
    fn full_verified_identities_leave_room_for_tab_controls() {
        let mut app = headless(900.0, 600.0);
        let destination = *yami_identity::NodeIdentity::generate().unwrap().id();
        let relay = *yami_identity::NodeIdentity::generate().unwrap().id();
        for _ in 1..crate::model::MAX_TABS {
            app.browser.open_tab();
        }
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        for security in [
            crate::model::Security::Verified(destination),
            crate::model::Security::Relayed { destination, relay },
        ] {
            if let PageState::Ready(page) = &mut app.browser.active_mut().state {
                page.security = security;
            }
            frame(&mut app);
            for hit in app.scene.hits.iter().filter(|h| {
                matches!(
                    h.action,
                    Action::Tab(_) | Action::Close(_) | Action::OpenGame
                )
            }) {
                assert!(
                    hit.rect.y + hit.rect.h < 600.0 - 188.0,
                    "{:?} at {:?}",
                    hit.action,
                    hit.rect
                );
            }
        }
    }

    #[test]
    fn document_layout_errors_use_the_same_scrollable_recovery_view() {
        let mut app = headless(900.0, 600.0);
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        frame(&mut app);
        app.native.as_mut().unwrap().document = Err("Detailed layout failure. ".repeat(100));
        frame(&mut app);
        assert!(app.scroll_limit() > 0.0);
        click_action(&mut app, Action::Navigate(""), true);
        assert!(matches!(app.browser.active().state, PageState::Home));
    }

    struct DiscoverFixture(std::path::PathBuf);
    impl Drop for DiscoverFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn discover_fixture(name: &str) -> (Desktop, DiscoverFixture) {
        use yami_discover::{Directory, DiscoveryStore};
        let root =
            std::env::temp_dir().join(format!("browser-discover-ui-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("community.ylang");
        let doc =
            Directory::parse(include_str!("../../../sites/directories/community.ylang")).unwrap();
        doc.save_new(&source).unwrap();
        let state = root.join("state");
        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("saved-community", &source).unwrap();
        store.refresh_source("saved-community").unwrap();
        store
            .register_remote("remote-directory", doc.entries()[0].address().clone())
            .unwrap();
        let mut app = headless(1000.0, 700.0);
        app.browser.library =
            Some(yami_browser::library::Library::open(root.join("library")).unwrap());
        app.settings.local_only = true;
        app.loader.set_local_only(true);
        app.discover = yami_browser::discover::Discover::new(Ok(state), Arc::new(|| {}));
        (app, DiscoverFixture(root))
    }
    fn settle_discover(app: &mut Desktop) {
        let until = Instant::now() + Duration::from_secs(10);
        while app.discover.busy.is_some() {
            app.discover.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn settle_pages(app: &mut Desktop) {
        let until = Instant::now() + Duration::from_secs(10);
        while app.discover.pages.busy.is_some() {
            app.discover.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn pages_mode_native_controls_cached_results_and_directory_compatibility() {
        use yami_browser::discover::Field;
        use yami_search::{Index, Page, Publication, SearchStore, WebsiteAddress};
        let (mut app, fixture) = discover_fixture("pages");
        let state = fixture.0.join("state/pages");
        let id = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re";
        let pages = (0..6)
            .map(|i| Page {
                key: format!("p{i}"),
                title: format!("Café page {i}"),
                headings: String::new(),
                text: "Searchable original text".into(),
                address: Some(format!("yami://{id}/p{i}")),
            })
            .collect();
        let publication = Publication::build(&Index::from_pages(pages).unwrap()).unwrap();
        struct CachedFixture(Publication);
        impl yami_search::AuthenticatedDocumentFetcher for CachedFixture {
            fn fetch_document(
                &self,
                address: &WebsiteAddress,
            ) -> Result<ynp::DocumentData, yami_discover::RemoteFetchError> {
                let source = if address.key() == "search-manifest" {
                    self.0.manifest()
                } else {
                    &self
                        .0
                        .shards()
                        .iter()
                        .find(|(k, _)| k == address.key())
                        .unwrap()
                        .1
                };
                Ok(ynp::DocumentData {
                    key: address.key().into(),
                    source: source.into(),
                    style: String::new(),
                })
            }
        }
        let mut store = SearchStore::open(&state).unwrap();
        store
            .add(
                "Independent",
                WebsiteAddress::parse(&format!("yami://{id}/search-manifest")).unwrap(),
            )
            .unwrap();
        store
            .refresh(
                "Independent",
                &CachedFixture(publication),
                &std::sync::atomic::AtomicBool::new(false),
            )
            .unwrap();
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        let tabs = app.browser.tabs.len();
        app.act(Action::Discover);
        settle_discover(&mut app);
        assert_eq!(app.discover.listings.len(), 4);
        app.act(Action::DiscoverMode(true));
        settle_pages(&mut app);
        app.act(Action::DiscoverField(Field::Search));
        app.editor.replace("CAFÉ");
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        settle_pages(&mut app);
        assert_eq!(app.discover.pages.hits.len(), 6);
        assert_eq!(app.discover.pages.query, "CAFÉ");
        assert_eq!(app.browser.tabs.len(), tabs);
        app.keyboard(&Key::Named(NamedKey::ArrowDown), None, None);
        assert!(app.discover.pages.selected_page.is_some());
        for (width, height, scale) in [
            (800.0, 600.0, 1.0),
            (1000.0, 700.0, 1.0),
            (1440.0, 900.0, 2.0),
        ] {
            app.headless = Some((width, height, scale));
            for directories in [false, true] {
                app.act(Action::DiscoverView(directories));
                let mut pix = Pixmap::new((width * scale) as u32, (height * scale) as u32).unwrap();
                app.compose(&mut pix);
                let panel = chrome::discover_rect(width, height);
                for hit in app
                    .scene
                    .hits
                    .iter()
                    .filter(|h| h.rect.x >= chrome::SIDEBAR_WIDTH)
                {
                    assert!(
                        hit.rect.y >= panel.y
                            && hit.rect.x + hit.rect.w <= panel.x + panel.w
                            && hit.rect.y + hit.rect.h <= panel.y + panel.h,
                        "{:?} outside Pages panel",
                        hit.action
                    );
                }
                if let Some(capture) = std::env::var_os("YAMINET_DISCOVER_CAPTURE") {
                    let path = std::path::PathBuf::from(capture);
                    std::fs::create_dir_all(&path).unwrap();
                    pix.save_png(path.join(format!("pages-{width}-{directories}.png")))
                        .unwrap();
                }
            }
        }
        app.act(Action::DiscoverSource("Independent".into()));
        app.act(Action::DiscoverRefresh);
        assert!(app
            .discover
            .pages
            .error
            .as_deref()
            .unwrap()
            .contains("local-only"));
        assert!(app.discover.pages.busy.is_none());
        app.act(Action::DiscoverView(false));
        frame(&mut app);
        let action = app
            .scene
            .hits
            .iter()
            .find_map(|h| matches!(h.action, Action::DiscoverWebsite(_)).then(|| h.action.clone()))
            .unwrap();
        app.act(action);
        assert!(app.discover.pages.selected_page.is_some());
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        assert!(!app.discover.open);
        assert_eq!(app.browser.tabs.len(), tabs);
        assert!(app.browser.active().address.starts_with("yami://"));
        app.act(Action::Discover);
        settle_pages(&mut app);
        app.act(Action::DiscoverMode(false));
        settle_discover(&mut app);
        assert_eq!(app.discover.listings.len(), 4);
        assert_eq!(app.discover.sources.len(), 2);
    }
    #[test]
    fn discover_search_editor_selection_and_open_preserve_tabs_and_privacy() {
        use yami_browser::discover::Field;
        let (mut app, _fixture) = discover_fixture("editing");
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        app.act(Action::NewTab);
        app.act(Action::Navigate("local:about"));
        settle(&mut app);
        let address = app.browser.active().address.clone();
        let title = app.browser.active().title().to_string();
        app.act(Action::Discover);
        assert!(app.discover.busy.is_some());
        frame(&mut app);
        settle_discover(&mut app);
        assert_eq!(app.discover.listings.len(), 4);
        app.act(Action::DiscoverField(Field::Search));
        app.keyboard(&Key::Character("CAFÉ".into()), Some("CAFÉ"), None);
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        assert_eq!(app.discover.query, "CAFÉ");
        assert_eq!(app.discover.listings.len(), 1);
        assert_eq!(app.discover.listings[0].entry.title(), "Café & Code");
        assert_eq!(app.browser.active().address, address);
        assert_eq!(app.browser.active().title(), title);
        assert_eq!(app.browser.tabs.len(), 2);
        assert_eq!(
            app.browser.active().security(),
            Some(&crate::model::Security::BuiltIn)
        );
        app.act(Action::DiscoverWebsite(
            app.discover.listings[0].entry.address().url(),
        ));
        frame(&mut app);
        assert!(app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::DiscoverOpen));
        app.act(Action::DiscoverView(true));
        frame(&mut app);
        app.act(Action::DiscoverField(Field::Name));
        app.editor.replace("My café directory");
        frame(&mut app);
        app.keyboard(&Key::Named(NamedKey::Tab), None, None);
        assert_eq!(app.discover.name, "My café directory");
        assert_eq!(
            app.scene.hits[app.keyboard_focus.unwrap()].action,
            Action::DiscoverField(Field::Address)
        );
        app.keyboard(&Key::Named(NamedKey::Enter), None, None);
        app.editor.replace("https://invalid.example");
        app.act(Action::DiscoverAdd); // commits the active editor before validating
        assert!(app.discover.error.is_some());
        assert!(app.discover.busy.is_none());
        app.act(Action::DiscoverSource("remote-directory".into()));
        frame(&mut app);
        assert!(!app
            .scene
            .hits
            .iter()
            .any(|hit| hit.action == Action::DiscoverRefresh));
        app.act(Action::DiscoverRefresh);
        assert!(app
            .discover
            .error
            .as_deref()
            .unwrap()
            .contains("local-only"));
        assert!(app.discover.busy.is_none());
        app.act(Action::DiscoverView(false));
        app.act(Action::DiscoverWebsite(
            app.discover.listings[0].entry.address().url(),
        ));
        app.act(Action::DiscoverOpen);
        settle(&mut app);
        assert!(!app.discover.open);
        assert!(matches!(app.browser.active().state, PageState::Error(_)));
        assert!(app.browser.active().security().is_none());
        assert_eq!(app.browser.tabs.len(), 2);
        assert!(app.settings.local_only);
        assert!(!app.browser.library.as_ref().unwrap().history_enabled());
        assert!(app.browser.library.as_ref().unwrap().visits().is_empty());
        app.act(Action::Discover);
        settle_discover(&mut app);
        app.act(Action::DiscoverField(Field::Name));
        app.editor.replace("Saved draft");
        app.act(Action::Address);
        assert!(!app.discover.open && app.discover_edit.is_none());
        assert_eq!(app.discover.name, "Saved draft");
        assert_eq!(app.editor.value, app.browser.active().address);
    }
    #[test]
    fn discover_modal_geometry_pagination_cache_empty_and_error_states() {
        use yami_browser::discover::Field;
        let (mut app, fixture) = discover_fixture("states");
        app.act(Action::Navigate("local:welcome"));
        settle(&mut app);
        app.act(Action::Discover);
        settle_discover(&mut app);
        // Losing a local source retains its prior snapshot and reports cached.
        std::fs::remove_file(fixture.0.join("community.ylang")).unwrap();
        app.discover.select_source(0);
        app.act(Action::DiscoverRefresh);
        settle_discover(&mut app);
        assert!(matches!(
            app.discover.listings[0].state,
            yami_discover::SourceState::Cached { .. }
        ));
        frame(&mut app);
        let stale_card = app
            .scene
            .hits
            .iter()
            .find_map(|hit| {
                matches!(hit.action, Action::DiscoverWebsite(_)).then(|| hit.action.clone())
            })
            .unwrap();
        app.act(Action::DiscoverField(Field::Search));
        app.editor.replace("CAFE");
        app.act(stale_card);
        assert_eq!(app.discover.listings.len(), 1);
        assert!(
            app.discover.selected_website.is_none(),
            "an old card must never select a different result at the same index"
        );
        app.discover.set_field(Field::Search, String::new());
        for (width, height, scale) in [
            (800.0, 600.0, 1.0),
            (1000.0, 700.0, 1.0),
            (1440.0, 900.0, 2.0),
        ] {
            app.headless = Some((width, height, scale));
            for directories in [false, true] {
                app.act(Action::DiscoverView(directories));
                let mut pix = Pixmap::new((width * scale) as u32, (height * scale) as u32).unwrap();
                app.compose(&mut pix);
                let panel = chrome::discover_rect(width, height);
                for hit in app
                    .scene
                    .hits
                    .iter()
                    .filter(|hit| hit.rect.x >= chrome::SIDEBAR_WIDTH)
                {
                    assert!(
                        hit.rect.x >= panel.x
                            && hit.rect.y >= panel.y
                            && hit.rect.x + hit.rect.w <= panel.x + panel.w
                            && hit.rect.y + hit.rect.h <= panel.y + panel.h,
                        "{:?} outside panel",
                        hit.action
                    );
                    assert!(!matches!(hit.action, Action::Follow(_) | Action::Scrollbar));
                }
                if let Some(capture) = std::env::var_os("YAMINET_DISCOVER_CAPTURE") {
                    let path = std::path::PathBuf::from(capture);
                    std::fs::create_dir_all(&path).unwrap();
                    pix.save_png(path.join(format!("discover-{width}-{directories}.png")))
                        .unwrap();
                }
                assert!(
                    app.link().is_none(),
                    "modal panel blocks document hit testing"
                );
            }
        }
        app.headless = Some((800.0, 600.0, 1.0));
        app.act(Action::DiscoverView(false));
        app.keyboard(&Key::Named(NamedKey::PageDown), None, None);
        assert_eq!(app.discover.page, 1);
        app.discover
            .set_field(Field::Search, "No matching website".into());
        assert!(app.discover.listings.is_empty());
        assert_eq!(app.discover.page, 0);
        frame(&mut app);
        assert!(!app.scene.hits.iter().any(|hit| matches!(
            hit.action,
            Action::DiscoverOpen | Action::DiscoverWebsite(_)
        )));
        app.discover.set_field(Field::Search, "x".repeat(129));
        assert!(app.discover.query_error.is_some());
        frame(&mut app);
        app.keyboard(&Key::Named(NamedKey::Escape), None, None);
        assert!(!app.discover.open && app.discover_edit.is_none());
    }

    #[test]
    fn library_controls_edit_custom_titles_folders_and_home_without_trust() {
        let dir = std::env::temp_dir().join(format!("yami-ui-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = headless(1000.0, 700.0);
        app.browser.library = Some(yami_browser::library::Library::open(&dir).unwrap());
        app.browser.navigate("local:welcome", &app.loader).unwrap();
        settle(&mut app);
        app.act(Action::SaveBookmark);
        app.editor.replace("My start page");
        app.commit();
        assert_eq!(
            app.browser.library.as_ref().unwrap().bookmarks()[0].title,
            "My start page"
        );
        app.act(Action::EditBookmark(0, true));
        app.editor.replace("Local");
        app.commit();
        assert_eq!(
            app.browser.library.as_ref().unwrap().bookmarks()[0].folder,
            "Local"
        );
        frame(&mut app);
        assert!(app
            .scene
            .hits
            .iter()
            .any(|h| h.action == Action::OpenBookmark(0)));
        app.act(Action::SetHomepage);
        app.act(Action::NewTab);
        app.act(Action::Home);
        settle(&mut app);
        assert_eq!(app.browser.active().address, "local:welcome");
        assert_eq!(
            app.browser.active().security(),
            Some(&crate::model::Security::BuiltIn)
        );
        app.act(Action::ToggleHistory);
        assert!(app.browser.library.as_ref().unwrap().history_enabled());
        app.act(Action::ClearHistory);
        app.act(Action::RemoveBookmark(0));
        assert!(app.browser.library.as_ref().unwrap().bookmarks().is_empty());
        drop(app);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn editor_shift_selection_is_unicode_safe_and_replaces_only_range() {
        let mut editor = Editor::default();
        editor.activate("aé日z");
        editor.move_caret(1, false);
        editor.move_caret(3, true);
        assert_eq!(&editor.value[editor.range().unwrap()], "é");
        editor.replace("Ω");
        assert_eq!(editor.value, "aΩ日z");
        editor.move_caret(editor.value.len(), false);
        editor.move_caret(3, true);
        editor.backspace();
        assert_eq!(editor.value, "aΩ");
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
        app.settings_open = true;
        frame(&mut app)
            .save_png(dir.join("desktop-onboarding.png"))
            .unwrap();
        app.settings_open = false;
        app.act(Action::Find);
        app.editor.replace("YamiNet");
        frame(&mut app)
            .save_png(dir.join("desktop-find.png"))
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
