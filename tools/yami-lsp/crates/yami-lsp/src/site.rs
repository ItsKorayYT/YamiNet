//! Local website discovery and sandboxed page resolution.
//!
//! Definitions deliberately operate on paths only: the server never reads page
//! content to resolve a link and never performs a network request. Remote
//! `yami://`, `ynp://` and `ynps://` targets are not resolved here at all.

use std::path::{Path, PathBuf};

/// Maximum number of local page-key completions returned for one request.
pub const MAX_PAGE_KEYS: usize = 512;
/// Maximum directory entries inspected while building page-key completions.
const MAX_SCANNED_ENTRIES: usize = 4096;

/// A canonicalized local website directory.
///
/// The root is canonicalized on open, and every resolved page is canonicalized
/// and checked to remain underneath it. This rejects `..` traversal as well as
/// symlinks that point outside the site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteRoot {
    root: PathBuf,
}

impl SiteRoot {
    pub fn open(path: &Path) -> Option<Self> {
        let root = path.canonicalize().ok()?;
        if !root.is_dir() {
            return None;
        }
        Some(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a YLang local page key (`/about` -> `about.ylang`).
    ///
    /// Only syntactically valid, single-component keys are accepted. Missing
    /// files, directories, and symlinks resolving outside the site return
    /// `None`.
    pub fn resolve_page(&self, key: &str) -> Option<PathBuf> {
        if !ylang::valid_key(key) {
            return None;
        }
        let candidate = self.root.join(format!("{key}.ylang"));
        let resolved = candidate.canonicalize().ok()?;
        if !resolved.starts_with(&self.root) {
            return None;
        }
        if !resolved.is_file() {
            return None;
        }
        Some(resolved)
    }

    /// List local page keys for completion. Symlinks are skipped entirely so a
    /// completion can never advertise a page outside the site directory.
    pub fn page_keys(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut keys = Vec::new();
        for entry in entries.flatten().take(MAX_SCANNED_ENTRIES) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Some(key) = name.strip_suffix(".ylang") else {
                continue;
            };
            if ylang::valid_key(key) {
                keys.push(key.to_string());
            }
        }
        keys.sort();
        keys.dedup();
        keys.truncate(MAX_PAGE_KEYS);
        keys
    }
}

/// Extract a local page key from a YLang 0.2 link target. Returns `None` for
/// remote schemes, path traversal and every non-key target.
pub fn local_key(target: &str) -> Option<&str> {
    let key = target.strip_prefix('/')?;
    if ylang::valid_key(key) {
        Some(key)
    } else {
        None
    }
}

/// Convert a canonical/absolute path to a `file://` URI.
///
/// Percent-encodes everything except unreserved URI characters, `/` and the
/// Windows drive colon. Non-UTF-8 path components are not representable and
/// yield `None`.
pub fn path_to_file_uri(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let text = path.to_str()?.replace('\\', "/");
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    Some(uri)
}

/// Convert a local `file://` URI to a path. Remote/UNC authorities are rejected
/// so configuration cannot cause network file access.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri
        .get(..7)
        .filter(|prefix| prefix.eq_ignore_ascii_case("file://"))
        .map(|_| &uri[7..])?;
    if rest.is_empty() || rest.starts_with("//") || !rest.starts_with('/') {
        return None;
    }
    if rest.contains(['?', '#']) {
        return None;
    }
    let decoded = percent_decode(rest)?;
    let text = String::from_utf8(decoded).ok()?;
    if text.contains('\0') {
        return None;
    }
    #[cfg(windows)]
    {
        let stripped = text.strip_prefix('/')?;
        let bytes = stripped.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || !matches!(bytes[2], b'/' | b'\\')
        {
            return None;
        }
        let path = stripped.replace('/', "\\");
        Some(PathBuf::from(path))
    }
    #[cfg(not(windows))]
    {
        Some(PathBuf::from(text))
    }
}

fn percent_decode(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = *bytes.get(index + 1)?;
                let low = *bytes.get(index + 2)?;
                let high = (high as char).to_digit(16)?;
                let low = (low as char).to_digit(16)?;
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    Some(out)
}

/// Interpret a configuration value (raw path or file URI), resolving relative
/// paths against `base` when one is available.
pub fn path_from_setting(value: &str, base: Option<&Path>) -> Option<PathBuf> {
    if value.trim().is_empty() {
        return None;
    }
    let path = if value
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
    {
        file_uri_to_path(value)?
    } else {
        PathBuf::from(value)
    };
    Some(match (path.is_relative(), base) {
        (true, Some(base)) => base.join(path),
        _ => path,
    })
}

/// Directory containing a `file://` document, used as a last-resort site root
/// when the client opens a single file without a workspace.
pub fn document_directory(uri: &str) -> Option<PathBuf> {
    let path = file_uri_to_path(uri)?;
    path.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../..")
            .canonicalize()
            .unwrap()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("yami-lsp-{name}-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn resolves_real_sample_site_and_rejects_invalid_keys() {
        let blog = repo_root().join("sites/blog");
        let site = SiteRoot::open(&blog).unwrap();
        let about = site.resolve_page("about").unwrap();
        assert!(about.ends_with("about.ylang"));
        assert!(about.starts_with(site.root()));
        let keys = site.page_keys();
        assert!(keys.contains(&"about".to_string()));
        assert!(keys.contains(&"home".to_string()));
        assert!(!keys.contains(&"default".to_string()));

        for key in [
            "",
            ".",
            "..",
            "../home",
            "about/../home",
            "about.ylang",
            "/about",
            "about%2fhome",
            "about home",
            "about\\home",
        ] {
            assert!(site.resolve_page(key).is_none(), "{key:?}");
        }
        assert!(site.resolve_page("missing").is_none());
    }

    #[test]
    fn file_uri_round_trip_encodes_spaces_and_unicode() {
        let path = std::env::temp_dir().join("yami lsp café");
        let uri = path_to_file_uri(&path).unwrap();
        assert!(uri.starts_with("file:///"));
        assert!(uri.contains("yami%20lsp%20caf%C3%A9"));
        assert_eq!(file_uri_to_path(&uri).unwrap(), path);
        assert!(path_to_file_uri(Path::new("relative/path")).is_none());
    }

    #[test]
    fn remote_file_authorities_are_rejected() {
        assert!(file_uri_to_path("file://server/share/page.ylang").is_none());
        assert!(file_uri_to_path("file:///C:/site?x").is_none());
        assert!(file_uri_to_path("https://example.com/page").is_none());
        #[cfg(windows)]
        assert_eq!(
            file_uri_to_path("file:///C:/site/page.ylang").unwrap(),
            PathBuf::from(r"C:\site\page.ylang")
        );
        #[cfg(not(windows))]
        assert_eq!(
            file_uri_to_path("file:///tmp/site/page.ylang").unwrap(),
            PathBuf::from("/tmp/site/page.ylang")
        );
    }

    #[test]
    fn symlink_outside_site_is_never_resolved_or_completed() {
        let root = temp_dir("symlink-root");
        let outside = temp_dir("symlink-outside");
        let outside_page = outside.join("secret.ylang");
        std::fs::write(&outside_page, "@page 0.2\n  title Secret").unwrap();
        let link = root.join("leak.ylang");
        let created = create_file_symlink(&outside_page, &link);
        if !created {
            let _ = std::fs::remove_dir_all(&root);
            let _ = std::fs::remove_dir_all(&outside);
            return;
        }
        let site = SiteRoot::open(&root).unwrap();
        assert!(site.resolve_page("leak").is_none());
        assert!(!site.page_keys().contains(&"leak".to_string()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    #[cfg(not(any(unix, windows)))]
    fn create_file_symlink(_target: &Path, _link: &Path) -> bool {
        false
    }
}
