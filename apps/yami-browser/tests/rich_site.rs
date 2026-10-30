//! Real native model, unchanged document frames, v2 PNG frames and pinned TLS.
use std::{
    fs, io,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use yami_browser::{
    model::{Browser, Loader, PageState, Security},
    native::Resolver,
};
use yami_dht::{Network, Service};
use yami_identity::NodeIdentity;
use yami_node::{shutdown::Shutdown, Documents, Site};
use yami_render::{
    assets::{self, ImageState},
    native::{NativeDocument, TextEngine},
};
struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("yami-rich-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("site/assets")).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Tracked {
    site: Site,
    assets: Arc<AtomicUsize>,
    slow: Arc<AtomicBool>,
}
impl Documents for Tracked {
    fn lookup(&self, key: &str) -> io::Result<Option<ynp::DocumentData>> {
        self.site.lookup(key)
    }
    fn asset(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        self.assets.fetch_add(1, Ordering::SeqCst);
        if self.slow.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(600));
        }
        self.site.asset(key)
    }
}
struct Host {
    id: yami_identity::NodeId,
    endpoint: SocketAddr,
    stop: Shutdown,
    thread: Option<thread::JoinHandle<()>>,
    requests: Arc<AtomicUsize>,
    slow: Arc<AtomicBool>,
}
impl Host {
    fn start(scratch: &Scratch) -> Self {
        let identity = Arc::new(NodeIdentity::create(scratch.0.join("identity")).unwrap());
        let id = *identity.id();
        let listener =
            ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), identity.clone()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = listener.local_addr().unwrap();
        let service = Service::open(scratch.0.join("host.cache"), id, Network::Loopback).unwrap();
        let SocketAddr::V4(ip) = endpoint else {
            unreachable!()
        };
        service
            .prepare_self(&identity, vec![ip], yami_dht::now().unwrap())
            .unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let slow = Arc::new(AtomicBool::new(false));
        let tracked = Tracked {
            site: Site::open(scratch.0.join("site")).unwrap(),
            assets: requests.clone(),
            slow: slow.clone(),
        };
        let stop = Shutdown::default();
        let signal = stop.clone();
        let thread = thread::spawn(move || {
            yami_node::serve_with_handler_until(
                &listener,
                4,
                &|connection| yami_node::serve_connection_with_dht(connection, &tracked, &service),
                &|_| {},
                &signal,
            )
            .unwrap()
        });
        Self {
            id,
            endpoint,
            stop,
            thread: Some(thread),
            requests,
            slow,
        }
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        self.stop.stop();
        self.thread.take().unwrap().join().unwrap();
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(8);
    while !ready() {
        assert!(Instant::now() < until, "rich site operation timed out");
        thread::sleep(Duration::from_millis(5));
    }
}
fn poll(browser: &mut Browser, loader: &Loader) {
    wait(|| {
        browser.poll(loader);
        !matches!(browser.active().state, PageState::Loading)
    })
}
fn done(browser: &mut Browser, loader: &Loader) {
    wait(|| {
        browser.poll(loader);
        matches!(&browser.active().state,PageState::Ready(page) if page.images.values().all(|image|!matches!(image,ImageState::Pending)))
    })
}
fn page(browser: &Browser) -> &yami_browser::model::Page {
    let PageState::Ready(page) = &browser.active().state else {
        panic!("{:?}", browser.active().state)
    };
    page
}
#[test]
fn native_identity_site_loads_rich_content_images_and_local_links_over_tls() {
    let scratch = Scratch::new("complete");
    fs::write(scratch.0.join("site/home.ylang"),"@page 0.2\n  title Rich homepage\n= Hello [strong: YamiNet]\nA [em: native] site with [code: original] technology.\n:: row\n  :: card Welcome\n    :: list\n      - Unicode café\n      - Private local bookmarks\n  :: card Explore\n    > /about | About this site\n! cover | A red landscape illustration\n! missing | Missing image fallback\n! corrupt | Corrupt image fallback\n---\n").unwrap();
    fs::write(scratch.0.join("site/about.ylang"),"@page 0.2\n  title About this site\n:: steps\n  + Write YLang\n  + Browse natively\n> /home | Home\n").unwrap();
    fs::write(
        scratch.0.join("site/default.ysl"),
        "@style 0.2\ncanvas\n  width 960\ncard\n  radius 18\nimage\n  width 320\n",
    )
    .unwrap();
    fs::write(
        scratch.0.join("site/assets/cover.png"),
        assets::encode_png(8, 4, &[220, 30, 50, 255].repeat(32)).unwrap(),
    )
    .unwrap();
    let mut corrupt = assets::encode_png(1, 1, &[0, 0, 0, 255]).unwrap();
    corrupt[29] ^= 1;
    fs::write(scratch.0.join("site/assets/corrupt.png"), corrupt).unwrap();
    let host = Host::start(&scratch);
    let directory = scratch.0.join("browser");
    yami_node::peers::PeerStore::open(&directory)
        .unwrap()
        .add(host.id, host.endpoint)
        .unwrap();
    let resolver = Arc::new(Resolver::new(directory, Network::Loopback));
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{}/home", host.id), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    done(&mut browser, &loader);
    assert!(matches!(
        page(&browser).images.get("cover"),
        Some(ImageState::Ready(_))
    ));
    assert!(matches!(
        page(&browser).images.get("missing"),
        Some(ImageState::Missing(_))
    ));
    assert!(matches!(
        page(&browser).images.get("corrupt"),
        Some(ImageState::Missing(_))
    ));
    assert_eq!(host.requests.load(Ordering::SeqCst), 3);
    let mut engine = TextEngine::new();
    let page = page(&browser);
    let layout = NativeDocument::layout_with_assets(
        &page.document,
        &page.style,
        800.0,
        1.0,
        &mut engine,
        &page.images,
    )
    .unwrap();
    let mut pixels = tiny_skia::Pixmap::new(800, 1000).unwrap();
    layout.draw(&mut pixels, 0.0, 0.0, 0.0, 1000.0, &mut engine);
    assert!(pixels
        .pixels()
        .iter()
        .any(|p| p.red() == 220 && p.green() == 30 && p.blue() == 50));
    browser.follow("/about", &loader).unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().address,
        format!("yami://{}/about", host.id)
    );
    assert_eq!(browser.active().title(), "About this site");
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    assert!(browser.back(&loader));
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert_eq!(browser.active().title(), "Rich homepage");
}
#[test]
fn slow_assets_do_not_block_navigation_and_cancellation_stops_remaining_requests() {
    let scratch = Scratch::new("cancel");
    fs::write(
        scratch.0.join("site/home.ylang"),
        format!(
            "@page 0.2\n  title Slow assets\n{}",
            (0..12)
                .map(|i| format!("! image{i} | An image\n"))
                .collect::<String>()
        ),
    )
    .unwrap();
    let host = Host::start(&scratch);
    host.slow.store(true, Ordering::SeqCst);
    let loader = Loader::new(Arc::new(|| {})).unwrap();
    let mut browser = Browser::new();
    let url = ynp_tls::format_url(&host.id, host.endpoint, "home");
    browser.navigate(&url, &loader).unwrap();
    poll(&mut browser, &loader);
    wait(|| host.requests.load(Ordering::SeqCst) > 0);
    let start = Instant::now();
    browser.navigate("local:welcome", &loader).unwrap();
    poll(&mut browser, &loader);
    assert!(start.elapsed() < Duration::from_millis(400));
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
    thread::sleep(Duration::from_millis(650));
    browser.poll(&loader);
    assert_eq!(host.requests.load(Ordering::SeqCst), 1);
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
}
#[test]
fn distinct_image_attempts_are_bounded_and_plaintext_parent_never_fetches_images() {
    let scratch = Scratch::new("bounded");
    fs::write(
        scratch.0.join("site/home.ylang"),
        format!(
            "@page 0.2\n  title Budget\n{}",
            (0..12)
                .map(|i| format!("! image{i} | An image\n"))
                .collect::<String>()
        ),
    )
    .unwrap();
    let host = Host::start(&scratch);
    let loader = Loader::new(Arc::new(|| {})).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(
            &ynp_tls::format_url(&host.id, host.endpoint, "home"),
            &loader,
        )
        .unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert_eq!(
        host.requests.load(Ordering::SeqCst),
        assets::MAX_PAGE_IMAGES
    );
    let site = Site::open(scratch.0.join("site")).unwrap();
    let listener = ynp::LocalListener::bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let plain = thread::spawn(move || yami_node::serve_once(&listener, &site).unwrap());
    browser
        .navigate(&format!("ynp://{endpoint}/home"), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert_eq!(browser.active().security(), Some(&Security::Plaintext));
    assert!(page(&browser).images.values().all(
        |image| matches!(image,ImageState::Missing(error) if error.contains("authenticated parent"))
    ));
    plain.join().unwrap();
    assert_eq!(
        host.requests.load(Ordering::SeqCst),
        assets::MAX_PAGE_IMAGES
    );
}

#[test]
fn png_assets_keep_nested_tls_destination_pinning_through_a_relay() {
    let scratch = Scratch::new("relay-assets");
    fs::write(scratch.0.join("site/home.ylang"),"@page 0.2\n  title Relayed images\n! cover | Relayed green illustration\n> /about | About\n").unwrap();
    fs::write(
        scratch.0.join("site/about.ylang"),
        "@page 0.2\n  title Relayed about\n= Still the same identity\n",
    )
    .unwrap();
    fs::write(
        scratch.0.join("site/assets/cover.png"),
        assets::encode_png(4, 2, &[20, 180, 70, 255].repeat(8)).unwrap(),
    )
    .unwrap();
    let identity = Arc::new(NodeIdentity::create(scratch.0.join("host-keys")).unwrap());
    let host = *identity.id();
    let relay_key = Arc::new(NodeIdentity::generate().unwrap());
    let relay = Arc::new(
        yami_relay::Relay::bind(
            "127.0.0.1:0".parse().unwrap(),
            relay_key.clone(),
            yami_relay::Limits::default(),
        )
        .unwrap(),
    );
    let endpoint = relay.local_addr().unwrap();
    let stop = Shutdown::default();
    let relay_stop = stop.clone();
    let running = relay.clone();
    let relay_task = thread::spawn(move || running.run(relay_stop.flag()).unwrap());
    let route = yami_relay::Route {
        relay: *relay_key.id(),
        endpoint,
        destination: host,
    };
    let signal = stop.clone();
    let site = Site::open(scratch.0.join("site")).unwrap();
    let host_task = thread::spawn(move || {
        yami_relay::run_host(
            route,
            identity,
            signal.flag(),
            &|connection| yami_node::serve_connection(connection, &site).map(|_| ()),
            &|_| {},
        )
        .unwrap()
    });
    struct Tasks(Shutdown, Vec<thread::JoinHandle<()>>);
    impl Drop for Tasks {
        fn drop(&mut self) {
            self.0.stop();
            for task in self.1.drain(..) {
                task.join().unwrap();
            }
        }
    }
    let _tasks = Tasks(stop, vec![relay_task, host_task]);
    wait(|| relay.statistics().registrations == 1);
    let owner = NodeIdentity::load(scratch.0.join("host-keys")).unwrap();
    let now = yami_dht::now().unwrap();
    let SocketAddr::V4(endpoint) = endpoint else {
        unreachable!()
    };
    let record = yami_dht::PeerRecord::sign_routes(
        &owner,
        vec![],
        vec![yami_dht::RelayRoute {
            relay: *relay_key.id(),
            endpoint,
        }],
        now + 120,
        1,
        Network::Loopback,
        now,
    )
    .unwrap();
    let resolver = Arc::new(Resolver::new(
        scratch.0.join("relay-client"),
        Network::Loopback,
    ));
    resolver.set_relay_enabled(true);
    let cache = Service::open_lookup(resolver.state_path(), Network::Loopback).unwrap();
    cache.accept(record, now).unwrap();
    drop(cache);
    let loader = Loader::with_resolver(Arc::new(|| {}), resolver).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{host}/home"), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert!(matches!(
        page(&browser).images.get("cover"),
        Some(ImageState::Ready(_))
    ));
    assert_eq!(
        browser.active().security(),
        Some(&Security::Relayed {
            destination: host,
            relay: *relay_key.id()
        })
    );
    browser.follow("/about", &loader).unwrap();
    poll(&mut browser, &loader);
    assert_eq!(browser.active().address, format!("yami://{host}/about"));
    assert_eq!(browser.active().title(), "Relayed about");
}

#[test]
fn authored_site_build_identity_bookmark_restart_and_native_render() {
    let scratch = Scratch::new("authoring");
    fs::remove_dir_all(scratch.0.join("site")).unwrap();
    let source = scratch.0.join("source");
    yami_node::authoring::new_site(&source, "portfolio").unwrap();
    yami_node::authoring::build(&source, scratch.0.join("site")).unwrap();
    let host = Host::start(&scratch);
    let loader = Loader::new(Arc::new(|| {})).unwrap();
    let url = ynp_tls::format_url(&host.id, host.endpoint, "home");
    let library = scratch.0.join("library");
    let mut browser = Browser::new();
    browser.library = Some(yami_browser::library::Library::open(&library).unwrap());
    browser.library.as_mut().unwrap().set_history(true).unwrap();
    browser.navigate(&url, &loader).unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    browser
        .bookmark_current("My portfolio", "Projects")
        .unwrap();
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    let p = page(&browser);
    let mut engine = TextEngine::new();
    for width in [340, 1000] {
        let layout = NativeDocument::layout_with_assets(
            &p.document,
            &p.style,
            width as f32,
            1.0,
            &mut engine,
            &p.images,
        )
        .unwrap();
        let mut pix = tiny_skia::Pixmap::new(width, 1600).unwrap();
        layout.draw(&mut pix, 0.0, 0.0, 0.0, 1600.0, &mut engine);
        assert!(pix
            .pixels()
            .iter()
            .any(|p| p.red() == 36 && p.green() == 72 && p.blue() == 85));
        if width == 1000 {
            if let Some(path) = std::env::var_os("YAMINET_CAPTURE_SITE") {
                pix.save_png(path).unwrap();
            }
        }
    }
    browser.follow("/about", &loader).unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().address,
        ynp_tls::format_url(&host.id, host.endpoint, "about")
    );
    assert_eq!(browser.library.as_ref().unwrap().visits().len(), 2);
    drop(browser);
    let mut browser = Browser::new();
    browser.library = Some(yami_browser::library::Library::open(&library).unwrap());
    let bookmark = browser.library.as_ref().unwrap().bookmarks()[0].clone();
    assert_eq!(bookmark.title, "My portfolio");
    assert_eq!(bookmark.address, url);
    assert!(browser.active().security().is_none());
    browser.navigate(&bookmark.address, &loader).unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(host.id))
    );
    browser
        .library
        .as_mut()
        .unwrap()
        .set_history(false)
        .unwrap();
    browser.library.as_mut().unwrap().clear_history().unwrap();
    browser.reload(&loader);
    poll(&mut browser, &loader);
    assert!(browser.library.as_ref().unwrap().visits().is_empty());
}

#[test]
fn assets_reauthenticate_exact_parent_even_when_endpoint_changes_identity() {
    let scratch = Scratch::new("asset-impostor");
    fs::write(
        scratch.0.join("site/home.ylang"),
        "@page 0.2\n  title Authenticated parent\n! cover | Identity must match the parent\n",
    )
    .unwrap();
    let site = Site::open(scratch.0.join("site")).unwrap();
    let identity = Arc::new(NodeIdentity::generate().unwrap());
    let id = *identity.id();
    let wrong = Arc::new(NodeIdentity::generate().unwrap());
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let accept = ynp_tls::TlsAcceptor::new(identity).unwrap();
        let (socket, _) = listener.accept().unwrap();
        let mut connection = accept
            .accept_over(Box::new(socket), Duration::from_secs(3))
            .unwrap();
        yami_node::serve_connection(&mut *connection, &site).unwrap();
        drop(connection);
        let (socket, _) = listener.accept().unwrap();
        let accept = ynp_tls::TlsAcceptor::new(wrong).unwrap();
        assert!(accept
            .accept_over(Box::new(socket), Duration::from_secs(3))
            .is_err());
    });
    let loader = Loader::new(Arc::new(|| {})).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&ynp_tls::format_url(&id, endpoint, "home"), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    done(&mut browser, &loader);
    assert_eq!(browser.active().security(), Some(&Security::Verified(id)));
    assert!(matches!(
        page(&browser).images.get("cover"),
        Some(ImageState::Missing(_))
    ));
    server.join().unwrap();
}

#[test]
fn explicit_cross_identity_links_require_navigation_and_reauthentication() {
    let a = Scratch::new("cross-a");
    let b = Scratch::new("cross-b");
    fs::write(
        b.0.join("site/home.ylang"),
        "@page 0.2\n  title Other identity\n= Other identity\n",
    )
    .unwrap();
    let bh = Host::start(&b);
    fs::write(
        a.0.join("site/home.ylang"),
        format!(
            "@page 0.2\n  title First identity\n> yami://{}/home | Other site\n",
            bh.id
        ),
    )
    .unwrap();
    let ah = Host::start(&a);
    let directory = a.0.join("peers");
    {
        let mut peers = yami_node::peers::PeerStore::open(&directory).unwrap();
        peers.add(ah.id, ah.endpoint).unwrap();
        peers.add(bh.id, bh.endpoint).unwrap();
    }
    let loader = Loader::with_resolver(
        Arc::new(|| {}),
        Arc::new(Resolver::new(directory, Network::Loopback)),
    )
    .unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(&format!("yami://{}/home", ah.id), &loader)
        .unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(ah.id))
    );
    assert_eq!(browser.active().history.len(), 2);
    let target = format!("yami://{}/home", bh.id);
    browser.follow(&target, &loader).unwrap();
    poll(&mut browser, &loader);
    assert_eq!(
        browser.active().security(),
        Some(&Security::Verified(bh.id))
    );
    assert_eq!(browser.active().address, target);
}

#[test]
fn closing_a_tab_cancels_its_remaining_image_work() {
    let scratch = Scratch::new("close");
    fs::write(
        scratch.0.join("site/home.ylang"),
        "@page 0.2\n  title Closing\n! first | First\n! second | Second\n",
    )
    .unwrap();
    let host = Host::start(&scratch);
    host.slow.store(true, Ordering::SeqCst);
    let loader = Loader::new(Arc::new(|| {})).unwrap();
    let mut browser = Browser::new();
    browser
        .navigate(
            &ynp_tls::format_url(&host.id, host.endpoint, "home"),
            &loader,
        )
        .unwrap();
    poll(&mut browser, &loader);
    wait(|| host.requests.load(Ordering::SeqCst) > 0);
    browser.open_tab();
    browser.close_tab_with_loader(0, &loader);
    browser.navigate("local:welcome", &loader).unwrap();
    poll(&mut browser, &loader);
    thread::sleep(Duration::from_millis(650));
    browser.poll(&loader);
    assert_eq!(host.requests.load(Ordering::SeqCst), 1);
    assert_eq!(browser.active().security(), Some(&Security::BuiltIn));
}
