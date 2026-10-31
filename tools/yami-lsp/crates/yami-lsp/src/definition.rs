//! Go-to-definition for local YLang page links.

use std::path::PathBuf;

use crate::site::{local_key, SiteRoot};
use crate::spec::{Language, Version};
use crate::text::TextDocument;

/// Resolve the local page link on the cursor's line.
///
/// Returns `None` for remote schemes, invalid keys, missing pages, unavailable
/// site roots and non-YLang 0.2 documents. No network access is ever attempted.
pub fn definition_at(
    document: &TextDocument,
    offset: usize,
    language: Language,
    version: Version,
    site: Option<&SiteRoot>,
) -> Option<PathBuf> {
    if language != Language::Ylang || version != Version::V02 {
        return None;
    }
    let site = site?;
    let line = document.position_at(offset).line;
    let text = document.line_text(line)?;
    let target = link_target(text.trim_start())?;
    let key = local_key(target)?;
    site.resolve_page(key)
}

fn link_target(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("> ")?;
    let (target, _label) = rest.split_once(" | ")?;
    let target = target.trim();
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn blog() -> SiteRoot {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../..");
        SiteRoot::open(&root.join("sites/blog")).unwrap()
    }

    fn resolve(
        text: &str,
        line: u32,
        version: Version,
        site: Option<&SiteRoot>,
    ) -> Option<PathBuf> {
        let document = TextDocument::new(text);
        let offset = document.offset_at(crate::text::Position::new(line, 0));
        definition_at(&document, offset, Language::Ylang, version, site)
    }

    #[test]
    fn resolves_valid_local_pages_in_real_sample_sites() {
        let site = blog();
        let text = "@page 0.2\n  title T\n> /about | About\n> /home | Home\n";
        let path = resolve(text, 2, Version::V02, Some(&site)).unwrap();
        assert!(path.ends_with("about.ylang"));
        let path = resolve(text, 3, Version::V02, Some(&site)).unwrap();
        assert!(path.ends_with("home.ylang"));
        // The definition works with the cursor in the label too.
        assert!(resolve(text, 2, Version::V02, Some(&site)).is_some());
    }

    #[test]
    fn rejects_missing_pages_traversal_and_remote_targets() {
        let site = blog();
        for line in [
            "> /missing | Missing",
            "> /../home | Traversal",
            "> /about/../home | Traversal",
            "> //home | Empty segment",
            "> /about.ylang | Extension",
            "> /about?x | Query",
            "> /about#x | Fragment",
            "> yami://yn1-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/home | Remote",
            "> ynp://127.0.0.1:7302/home | Remote",
            "> http://example.com/about | Remote",
            "> | Empty",
        ] {
            let text = format!("@page 0.2\n  title T\n{line}\n");
            assert!(
                resolve(&text, 2, Version::V02, Some(&site)).is_none(),
                "{line}"
            );
        }
        assert!(resolve(
            "@page 0.2\n  title T\n> /about | About\n",
            2,
            Version::V02,
            None
        )
        .is_none());
    }

    #[test]
    fn ylang_01_has_no_local_page_links() {
        let site = blog();
        assert!(resolve(
            "@page 0.1\n  title T\n> /about | About\n",
            2,
            Version::V01,
            Some(&site)
        )
        .is_none());
        assert!(resolve(
            "@page 0.1\n  title T\n> ynp://127.0.0.1:7302/about | About\n",
            2,
            Version::V01,
            Some(&site)
        )
        .is_none());
    }
}
