//! Directory-backed site: `<root>/<key>.ylang` pages sharing `<root>/default.ysl`.
//!
//! Files are read on each request, so edits appear without restarting the node.
//! A key never becomes a path by itself: it must pass YNP key validation, and the
//! resolved file must be a regular file whose canonical path is exactly
//! `<canonical root>/<key>.ylang`. Symlinks, case aliases, subdirectories and
//! device names therefore cannot reach anything outside the listed pages.
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};
use ynp::DocumentData;

pub const PAGE_EXTENSION: &str = "ylang";
pub const STYLE_FILE: &str = "default.ysl";
/// Startup listings are informational; very large directories are truncated.
pub(crate) const MAX_LISTED: usize = 256;
/// Windows device names resolve to devices even with an extension on some versions.
const RESERVED: [&str; 4] = ["con", "prn", "aux", "nul"];

#[derive(Debug, Clone)]
pub struct Site {
    root: PathBuf,
}
impl Site {
    pub fn open(root: impl AsRef<Path>) -> io::Result<Self> {
        let root = fs::canonicalize(root)?;
        if !fs::metadata(&root)?.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "site root must be a directory",
            ));
        }
        Ok(Self { root })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Keys of the pages currently present, sorted. Content is not read.
    pub fn pages(&self) -> io::Result<Vec<String>> {
        let mut keys = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(key) = name.strip_suffix(&format!(".{PAGE_EXTENSION}")) else {
                continue;
            };
            if ynp::validate_key(key).is_ok() && !is_reserved(key) && entry.file_type()?.is_file() {
                keys.push(key.to_owned());
                if keys.len() == MAX_LISTED {
                    break;
                }
            }
        }
        keys.sort();
        Ok(keys)
    }
    /// `Ok(None)` means no such page (a YNP NotFound). Errors mean the page
    /// exists but cannot be served safely, and close the connection.
    pub fn lookup(&self, key: &str) -> io::Result<Option<DocumentData>> {
        ynp::validate_key(key)?;
        if is_reserved(key) {
            return Ok(None);
        }
        let Some(source) = self.read(&format!("{key}.{PAGE_EXTENSION}"), ynp::MAX_DOCUMENT)? else {
            return Ok(None);
        };
        Ok(Some(DocumentData {
            key: key.into(),
            source,
            style: self.style()?,
        }))
    }
    /// Restricted PNG keys only; never a caller-supplied filesystem path.
    pub fn asset(&self, key: &str) -> io::Result<Option<Vec<u8>>> {
        ynp::validate_asset_key(key)?;
        let directory = self.root.join("assets");
        match fs::symlink_metadata(&directory) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(meta) if !meta.file_type().is_dir() => {
                return Err(refused("assets", "is not a regular non-symlink directory"))
            }
            _ => {}
        }
        let name = format!("{key}.png");
        let path = directory.join(&name);
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(meta) if !meta.file_type().is_file() => {
                return Err(refused(&name, "is not a regular non-symlink file"))
            }
            _ => {}
        }
        let resolved = fs::canonicalize(&path)?;
        if resolved.parent() != Some(directory.as_path())
            || resolved.file_name() != Some(name.as_ref())
        {
            return Err(refused(
                &name,
                "resolves outside the assets directory or to an alias",
            ));
        }
        let mut bytes = Vec::new();
        let file = File::open(resolved)?;
        if !file.metadata()?.is_file() {
            return Err(refused(&name, "is not a regular file"));
        }
        file.take((ynp::MAX_ASSET_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > ynp::MAX_ASSET_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(refused(&name, "is not a bounded PNG image"));
        }
        Ok(Some(bytes))
    }
    pub fn assets(&self) -> io::Result<Vec<String>> {
        let directory = self.root.join("assets");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        if !fs::symlink_metadata(&directory)?.file_type().is_dir() {
            return Err(refused("assets", "is not a non-symlink directory"));
        }
        let mut keys = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if let Some(key) = name.strip_suffix(".png") {
                if ynp::validate_asset_key(key).is_ok() && entry.file_type()?.is_file() {
                    keys.push(key.to_string());
                    if keys.len() >= MAX_LISTED {
                        break;
                    }
                }
            }
        }
        keys.sort();
        Ok(keys)
    }
    /// The shared stylesheet. A missing one is allowed and served as empty;
    /// the browser then uses its defaults.
    pub fn style(&self) -> io::Result<String> {
        Ok(self.read(STYLE_FILE, ynp::MAX_STYLE)?.unwrap_or_default())
    }
    fn read(&self, name: &str, limit: usize) -> io::Result<Option<String>> {
        let path = self.root.join(name);
        // Canonicalization follows symlinks and, on Windows, reports the stored
        // letter case, so escapes and aliases fail this exact comparison.
        let resolved = match fs::canonicalize(&path) {
            Ok(resolved) => resolved,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if resolved.parent() != Some(self.root.as_path())
            || resolved.file_name() != Some(name.as_ref())
        {
            return Err(refused(
                name,
                "resolves outside the site or to another name",
            ));
        }
        let file = File::open(&resolved)?;
        if !file.metadata()?.is_file() {
            return Err(refused(name, "is not a regular file"));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(refused(name, "exceeds the YNP size limit"));
        }
        let text = String::from_utf8(bytes).map_err(|_| refused(name, "is not valid UTF-8"))?;
        // Some Windows editors prepend a byte-order mark; it is not content.
        Ok(Some(
            text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned(),
        ))
    }
}
fn is_reserved(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    RESERVED.contains(&key.as_str())
        || (key.len() == 4
            && (key.starts_with("com") || key.starts_with("lpt"))
            && key.as_bytes()[3].is_ascii_digit())
}
fn refused(name: &str, reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("refusing to serve {name}: it {reason}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_device_names() {
        for key in ["con", "NUL", "Com1", "lpt9", "aux"] {
            assert!(is_reserved(key), "{key}");
        }
        for key in ["console", "com", "com10", "home", "lptx"] {
            assert!(!is_reserved(key), "{key}");
        }
    }
}
