use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use yami_search::{Index, MAX_QUERY_CHARS};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-search-local-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn page(&self, key: &str, source: &str) {
        fs::write(self.0.join(format!("{key}.ylang")), source).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ranking_unicode_snippets_and_explicit_rebuild_persist() {
    let site = Scratch::new();
    let state = Scratch::new();
    site.page("title", "@page 0.1\n  title Café Networks\n= An introduction\nDecentralized networks connect people.\n");
    site.page("body", "@page 0.2\n  title Elsewhere\n:: card Notes\n  A [strong: CAFÉ] about networks.\n  :: list\n    - 日本語の手帖\n    - Straße\n  > /home | Visible label\n  ! photo | Flower alternative\n");
    fs::write(site.0.join("private.key"), "never-index-this").unwrap();
    fs::write(site.0.join("default.ysl"), "never-index-style").unwrap();
    let index = Index::build(&site.0, None).unwrap();
    assert_eq!(index.search("CAFE", 10).unwrap()[0].page.key, "title");
    let hits = index.search("cafe networks", 10).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(index.search("日本", 10).unwrap().len(), 1);
    assert!(index.search("strasse", 10).unwrap()[0]
        .snippet
        .contains("Straße"));
    assert!(index.search("visible label", 10).unwrap()[0]
        .snippet
        .contains("Visible label"));
    assert_eq!(index.search("alternative", 10).unwrap().len(), 1);
    assert!(index.search("never", 10).unwrap().is_empty());
    assert!(index.search("strong", 10).unwrap().is_empty());
    assert!(index.search("CAFE\u{301}", 10).unwrap().len() == 2);
    assert!(hits.iter().all(|h| h.page.address.is_none()));
    assert!(index.search(&"x".repeat(MAX_QUERY_CHARS + 1), 10).is_err());
    assert_eq!(index.search("net", 1).unwrap().len(), 1);
    index.save(&state.0).unwrap();
    site.page(
        "added",
        "@page 0.2\n  title A new page\nFreshly added content\n",
    );
    let loaded = Index::open(&state.0).unwrap();
    assert!(loaded.search("fresh", 10).unwrap().is_empty());
    assert_eq!(loaded.search("cafe", 10).unwrap()[0].page.key, "title");
    loaded.rebuild().unwrap().save(&state.0).unwrap();
    assert_eq!(
        Index::open(&state.0)
            .unwrap()
            .search("fresh", 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn invalid_rebuild_leaves_previous_snapshot_and_invalid_versions_fail() {
    let site = Scratch::new();
    let state = Scratch::new();
    site.page("home", "@page 0.2\n  title Good\nValid content\n");
    Index::build(&site.0, None).unwrap().save(&state.0).unwrap();
    site.page("bad", "this is not YLang");
    assert!(Index::open(&state.0).unwrap().rebuild().is_err());
    assert_eq!(
        Index::open(&state.0)
            .unwrap()
            .search("valid", 10)
            .unwrap()
            .len(),
        1
    );
    let path = state.0.join("index.json");
    let data = fs::read_to_string(&path)
        .unwrap()
        .replace("\"version\":1", "\"version\":999");
    fs::write(&path, data).unwrap();
    assert!(Index::open(&state.0).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_and_traversal_cannot_index_outside_site() {
    let site = Scratch::new();
    let outside = Scratch::new();
    outside.page("secret", "@page 0.2\n  title Private\nPrivate text\n");
    std::os::unix::fs::symlink(outside.0.join("secret.ylang"), site.0.join("home.ylang")).unwrap();
    assert!(Index::build(&site.0, None).is_err());
    assert!(Index::build(site.0.join(".."), None).is_err());
}

#[test]
fn supplied_identity_and_heading_weight_are_preserved() {
    let site = Scratch::new();
    site.page("heading", "@page 0.2\n  title First\n= Searchable\n");
    site.page("body", "@page 0.1\n  title Second\nSearchable\n");
    let identity = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re"
        .parse()
        .unwrap();
    let index = Index::build(&site.0, Some(identity)).unwrap();
    let hits = index.search("search", 10).unwrap();
    assert_eq!(hits[0].page.key, "heading");
    assert_eq!(
        hits[0].page.address.as_ref().unwrap(),
        &format!("yami://{identity}/heading")
    );
}
