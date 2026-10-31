//! Explicit native-window smoke. Logical test actions, not physical input/IME.
use super::*;
use std::path::PathBuf;
pub(super) struct Smoke {
    output: PathBuf,
    phase: u8,
    started: Instant,
    last_frame: u64,
    reopened: bool,
    checks: Vec<&'static str>,
}
impl Smoke {
    pub(super) fn new(output: PathBuf, reopened: bool) -> Self {
        Self {
            output,
            phase: 0,
            started: Instant::now(),
            last_frame: 0,
            reopened,
            checks: Vec::new(),
        }
    }
    fn check(&mut self, ok: bool, name: &'static str) -> Result<(), String> {
        if !ok {
            return Err(format!("native smoke failed: {name}"));
        }
        self.checks.push(name);
        Ok(())
    }
    pub(super) fn tick(
        &mut self,
        app: &mut Desktop,
        events: &ActiveEventLoop,
    ) -> Result<bool, String> {
        if self.started.elapsed() > Duration::from_secs(25) {
            return Err("native smoke deadline exceeded".into());
        }
        if app.frames <= self.last_frame {
            app.redraw();
            return Ok(false);
        }
        if self.phase > 0 && matches!(app.browser.active().state, PageState::Loading) {
            return Ok(false);
        }
        match self.phase {
            0 => {
                if !self.reopened {
                    if let Some(frame) = &app.frame {
                        frame
                            .save_png(self.output.with_file_name("onboarding-smoke.png"))
                            .map_err(|_| "onboarding capture failed")?;
                    }
                }
                self.check(
                    app.window.is_some() && app.surface.is_some(),
                    "native-window-and-frame",
                )?;
                let library = app
                    .browser
                    .library
                    .as_ref()
                    .ok_or("smoke needs an isolated writable library")?;
                self.check(
                    library.settings().onboarded == self.reopened,
                    "onboarding-persistence",
                )?;
                self.check(!library.history_enabled(), "history-default-off")?;
                if self.reopened {
                    self.check(
                        library.bookmarks().iter().any(|b| b.title == "Smoke café"),
                        "bookmark-reopened",
                    )?;
                }
                app.act(Action::FinishSetup);
                self.check(!app.settings_open, "first-run-save")?;
                app.act(Action::Navigate("local:welcome"));
            }
            1 => {
                self.check(
                    matches!(app.browser.active().state, PageState::Ready(_)),
                    "built-in-document",
                )?;
                let _ = app
                    .window
                    .as_ref()
                    .unwrap()
                    .request_inner_size(LogicalSize::new(1000.0, 700.0));
                app.act(Action::Find);
                app.editor.replace("YamiNet");
            }
            2 => {
                let (w, h, _) = app.size();
                self.check(
                    (w - 1000.0).abs() < 3.0 && (h - 700.0).abs() < 3.0,
                    "native-resize-event",
                )?;
                self.check(!app.find.matches.is_empty(), "find-highlight")?;
                app.act(Action::Address);
                app.editor.replace("local:about");
                app.commit();
            }
            3 => {
                self.check(
                    app.browser.active().address == "local:about",
                    "find-to-address-focus",
                )?;
                app.act(Action::Address);
                let mut clipboard =
                    arboard::Clipboard::new().map_err(|_| "clipboard unavailable")?;
                clipboard
                    .set_text("café naïve".to_string())
                    .map_err(|_| "clipboard write failed")?;
                app.clipboard = Some(clipboard);
                app.modifiers = ModifiersState::CONTROL;
                app.keyboard(&Key::Character("v".into()), None, None);
                self.check(app.editor.value == "café naïve", "native-clipboard-unicode")?;
                app.modifiers = ModifiersState::empty();
                app.clear_focus();
                app.find_open = false;
                for _ in 0..4 {
                    app.act(Action::NewTab);
                }
                for _ in 0..4 {
                    app.act(Action::Close(app.browser.active));
                }
                self.check(app.browser.tabs.len() == 1, "tabs-open-close")?;
                if let PageState::Ready(page) = &mut app.browser.active_mut().state {
                    page.document = ylang::parse(&format!(
                        "@page 0.2\n  title Smoke\n{}\ncafé target\n",
                        "Native long paragraph.\n\n".repeat(250)
                    ))
                    .map_err(|_| "long fixture parse")?;
                    page.revision += 1;
                }
            }
            4 => {
                self.check(app.scroll_limit() > 1000.0, "long-document-layout")?;
                app.act(Action::Find);
                app.editor.replace("café");
            }
            5 => {
                self.check(
                    !app.find.matches.is_empty() && app.browser.active().target_scroll > 1000.0,
                    "unicode-find-scroll",
                )?;
                app.act(Action::CloseFind);
                if let PageState::Ready(page) = &mut app.browser.active_mut().state {
                    page.document = ylang::parse(&format!(
                        "@page 0.2\n  title Images\n{}",
                        (0..8)
                            .map(|i| format!("! image{i} | Smoke artwork\n"))
                            .collect::<String>()
                    ))
                    .map_err(|_| "image fixture parse")?;
                    let png = include_bytes!("../../../../sites/showcase/assets/cover.png");
                    let image = Arc::new(
                        yami_render::assets::decode_png(png, yami_render::assets::MAX_DECODED)
                            .map_err(|_| "PNG fixture decode")?,
                    );
                    for i in 0..8 {
                        page.images.insert(
                            format!("image{i}"),
                            yami_render::assets::ImageState::Ready(image.clone()),
                        );
                    }
                    page.revision += 1;
                }
                let _ = app
                    .window
                    .as_ref()
                    .unwrap()
                    .request_inner_size(LogicalSize::new(1440.0, 900.0));
            }
            6 => {
                self.check(
                    app.current_layout().is_some(),
                    "image-heavy-render-after-resize",
                )?;
                app.browser
                    .bookmark_current("Smoke café", "Smoke")
                    .map_err(|_| "bookmark save failed")?;
                if let Some(frame) = &app.frame {
                    frame
                        .save_png(self.output.with_extension("png"))
                        .map_err(|_| "smoke screenshot save failed")?;
                }
                app.act(Action::Navigate("local:welcome"));
            }
            7 => {
                self.check(
                    app.browser.active().title() == "Welcome to YamiNet",
                    "rapid-navigation-recovery",
                )?;
                app.act(Action::Discover);
                app.act(Action::DiscoverMode(true));
            }
            8 => {
                if app.discover.pages.busy.is_some() {
                    return Ok(false);
                }
                self.check(
                    app.discover.open && app.discover.pages_mode,
                    "native-pages-mode",
                )?;
                app.discover
                    .set_field(yami_browser::discover::Field::Search, "cafe".into());
            }
            _ => {
                if app.discover.pages.busy.is_some() {
                    return Ok(false);
                }
                if std::env::var("YAMINET_SMOKE_FULLTEXT").as_deref() == Ok("1") {
                    self.check(
                        !app.discover.pages.hits.is_empty(),
                        "cached-fulltext-results",
                    )?;
                    app.keyboard(&Key::Named(NamedKey::ArrowDown), None, None);
                    self.check(
                        app.discover.pages.selected_page.is_some(),
                        "fulltext-keyboard-selection",
                    )?;
                }
                if let Some(frame) = &app.frame {
                    frame
                        .save_png(self.output.with_file_name("fulltext-smoke.png"))
                        .map_err(|_| "fulltext screenshot save failed")?;
                }
                self.write("PASS", app.frames)?;
                events.exit();
                return Ok(true);
            }
        }
        self.last_frame = app.frames;
        self.phase += 1;
        app.redraw();
        Ok(false)
    }
    pub(super) fn write(&self, result: &str, frames: u64) -> Result<(), String> {
        use std::io::Write;
        let report = serde_json::json!({"schema":"yaminet-native-window-smoke-1", "result":result, "version":env!("CARGO_PKG_VERSION"), "reopened":self.reopened, "frames":frames, "checks":self.checks, "input":"in-process logical actions and native clipboard; physical keyboard and IME NOT RUN"});
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.output)
            .map_err(|_| "smoke report must be a new writable file")?;
        file.write_all(report.to_string().as_bytes())
            .map_err(|_| "smoke report write failed".into())
    }
}
