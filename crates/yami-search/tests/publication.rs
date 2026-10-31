use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
};
use yami_search::{Index, Manifest, Page, Provider, Publication, MANIFEST_KEY};
use ynp::{Incoming, Transport};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "yami-search-publish-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn page(id: &str, key: &str, title: &str) -> Page {
    Page {
        key: key.into(),
        title: title.into(),
        headings: "Heading".into(),
        text: "Café 日本語 decentralized content".into(),
        address: Some(format!("yami://{id}/{key}")),
    }
}
fn identity() -> &'static str {
    "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re"
}

#[test]
fn two_sites_combine_deterministically_and_serve_with_existing_node() {
    let scratch = Scratch::new();
    let owner = Arc::new(yami_identity::NodeIdentity::generate().unwrap());
    let alice = Index::from_pages(vec![page(identity(), "home", "Alice")]).unwrap();
    let bob = Index::from_pages(vec![page(&owner.id().to_string(), "home", "Bob")]).unwrap();
    let state = scratch.0.join("provider");
    Provider::create(&state).unwrap();
    let mut provider = Provider::open(&state).unwrap();
    provider.add(&bob).unwrap();
    provider.add(&alice).unwrap();
    provider.add(&alice).unwrap();
    provider.save(&state).unwrap();
    let publication = Provider::open(&state).unwrap().build().unwrap();
    let first = publication.manifest().to_owned();
    let output = scratch.0.join("public");
    publication.export(&output).unwrap();
    assert_eq!(
        Publication::load(&output)
            .unwrap()
            .search("CAFE", 10)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(first, provider.build().unwrap().manifest());
    fs::write(output.join("keep.txt"), "unrelated").unwrap();
    assert!(provider.build().unwrap().export(&output).is_err());
    assert_eq!(
        fs::read_to_string(output.join("keep.txt")).unwrap(),
        "unrelated"
    );
    let listener =
        ynp_tls::TlsListener::bind("127.0.0.1:0".parse().unwrap(), owner.clone()).unwrap();
    let endpoint = listener.local_addr().unwrap();
    let site = yami_node::Site::open(&output).unwrap();
    let server = thread::spawn(move || {
        let mut connection = listener.accept().unwrap();
        yami_node::serve_connection(&mut *connection, &site).unwrap();
    });
    let mut connection = ynp_tls::TlsTransport::new(*owner.id())
        .unwrap()
        .connect(endpoint)
        .unwrap();
    let doc = ynp::request(&mut *connection, MANIFEST_KEY).unwrap();
    assert_eq!(doc.source, first);
    Manifest::parse(&doc.source).unwrap();
    server.join().unwrap();
}

#[test]
fn invalid_versions_structures_hashes_addresses_duplicates_and_incomplete_data_fail() {
    let index = Index::from_pages(vec![page(identity(), "home", "Café")]).unwrap();
    let publication = Publication::build(&index).unwrap();
    let manifest = Manifest::parse(publication.manifest()).unwrap();
    let sources: Vec<_> = publication
        .shards()
        .iter()
        .map(|(_, s)| s.clone())
        .collect();
    assert_eq!(manifest.assemble(&sources).unwrap().pages(), index.pages());
    assert!(Manifest::parse(
        &publication
            .manifest()
            .replace("search-manifest 1", "search-manifest 2")
    )
    .is_err());
    assert!(Manifest::parse(&publication.manifest().replace(":: bytes", ":: unknown")).is_err());
    assert!(Manifest::parse(
        &(publication.manifest().to_owned() + "  = Executable-looking heading\n")
    )
    .is_err());
    assert!(manifest.assemble(&[]).is_err());
    assert!(manifest
        .assemble(&[sources[0].replace("data ", "bad ")])
        .is_err());
    assert!(manifest.assemble(&[format!("{} ", sources[0])]).is_err());
    let mut bad = page(identity(), "home", "Bad");
    bad.address = Some(format!("yami://{}/../private", identity()));
    assert!(Index::from_pages(vec![bad]).is_err());
    let p = page(identity(), "home", "Duplicate");
    assert!(Index::from_pages(vec![p.clone(), p]).is_err());
    let scratch = Scratch::new();
    let state = scratch.0.join("provider");
    Provider::create(&state).unwrap();
    let mut provider = Provider::open(&state).unwrap();
    provider.add(&index).unwrap();
    assert!(provider
        .add(&Index::from_pages(vec![page(identity(), "home", "Conflicting")]).unwrap())
        .is_err());
    assert_eq!(provider.build().unwrap().manifest(), publication.manifest());
    let mut local = page(identity(), "local", "No identity");
    local.address = None;
    assert!(Publication::build(&Index::from_pages(vec![local]).unwrap()).is_err());
}

#[test]
fn multi_shard_utf8_splits_round_trip_under_actual_ylang_limits() {
    let mut pages = Vec::new();
    for n in 0..5 {
        let mut p = page(identity(), &format!("p{n}"), "日本語 Café");
        p.text = "日本語 Café systems ".repeat(1800);
        pages.push(p);
    }
    let index = Index::from_pages(pages).unwrap();
    let publication = Publication::build(&index).unwrap();
    assert!(publication.shards().len() > 1);
    for (_, doc) in publication.shards() {
        assert!(doc.len() <= ylang::MAX_SOURCE);
        ylang::parse(doc).unwrap();
    }
    let manifest = Manifest::parse(publication.manifest()).unwrap();
    let sources: Vec<_> = publication
        .shards()
        .iter()
        .map(|(_, s)| s.clone())
        .collect();
    assert_eq!(manifest.assemble(&sources).unwrap().pages(), index.pages());
}
