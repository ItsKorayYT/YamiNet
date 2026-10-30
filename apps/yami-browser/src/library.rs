//! Bounded, local-only browser preferences. Nothing here establishes trust.
use crate::model::Location;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_BOOKMARKS: usize = 256;
pub const MAX_HISTORY: usize = 512;
const MAX_FILE: usize = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    pub address: String,
    pub title: String,
    pub folder: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Visit {
    pub address: String,
    pub title: String,
    pub time: u64,
}
#[derive(Clone, Default)]
struct Data {
    bookmarks: Vec<Bookmark>,
    visits: Vec<Visit>,
    history: bool,
    home: String,
    settings: Settings,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub onboarded: bool,
    pub local_only: bool,
    pub relays: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            onboarded: false,
            local_only: true,
            relays: false,
        }
    }
}
pub struct Library {
    dir: PathBuf,
    data: Data,
    _lock: File,
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
fn field(s: &str, max: usize) -> io::Result<()> {
    if s.len() > max || s.chars().any(char::is_control) {
        Err(invalid(
            "library field is oversized or contains control characters",
        ))
    } else {
        Ok(())
    }
}
fn address(s: &str) -> io::Result<String> {
    field(s, 512)?;
    Location::parse(s)
        .map(|l| l.address())
        .map_err(|_| invalid("invalid library address"))
}
fn regular(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.file_type().is_file() => Err(invalid(
            "library files must be regular files, never symlinks",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        _ => Ok(()),
    }
}
impl Library {
    pub fn default_directory() -> io::Result<PathBuf> {
        if let Some(p) = std::env::var_os("YAMINET_BROWSER_DATA_DIR") {
            let p = PathBuf::from(p);
            if !p.is_absolute() {
                return Err(invalid("YAMINET_BROWSER_DATA_DIR must be absolute"));
            }
            return Ok(p);
        }
        Ok(yami_node::peers::default_dir()?.join("browser-library"))
    }
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        if !fs::symlink_metadata(dir)?.file_type().is_dir() {
            return Err(invalid("library directory must not be a symlink"));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        regular(&dir.join("library.lock"))?;
        let lock = options.open(dir.join("library.lock"))?;
        lock.try_lock()
            .map_err(|_| invalid("browser library is already open or locked"))?;
        regular(&dir.join("library.tsv"))?;
        let data = match File::open(dir.join("library.tsv")) {
            Ok(file) => {
                let mut text = String::new();
                file.take((MAX_FILE + 1) as u64).read_to_string(&mut text)?;
                parse(&text)?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Data::default(),
            Err(e) => return Err(e),
        };
        // Only the exclusive owner may remove an abandoned temporary file.
        regular(&dir.join("library.tmp"))?;
        match fs::remove_file(dir.join("library.tmp")) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self {
            dir: dir.into(),
            data,
            _lock: lock,
        })
    }
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.data.bookmarks
    }
    pub fn visits(&self) -> &[Visit] {
        &self.data.visits
    }
    pub fn history_enabled(&self) -> bool {
        self.data.history
    }
    pub fn homepage(&self) -> &str {
        &self.data.home
    }
    pub fn settings(&self) -> Settings {
        self.data.settings
    }
    pub fn set_settings(&mut self, settings: Settings) -> io::Result<()> {
        let mut data = self.data.clone();
        data.settings = settings;
        self.commit(data)
    }
    /// Reset preferences and optional visits; retain bookmarks, keys and peer stores.
    pub fn reset_settings(&mut self) -> io::Result<()> {
        let mut data = self.data.clone();
        data.settings = Settings::default();
        data.history = false;
        data.visits.clear();
        data.home.clear();
        self.commit(data)
    }
    pub fn save_bookmark(&mut self, url: &str, title: &str, folder: &str) -> io::Result<()> {
        let url = address(url)?;
        if url.is_empty() {
            return Err(invalid("open a page before saving a bookmark"));
        }
        field(title, 160)?;
        field(folder, 64)?;
        let mut data = self.data.clone();
        let item = Bookmark {
            address: url.clone(),
            title: title.into(),
            folder: folder.into(),
        };
        if let Some(old) = data.bookmarks.iter_mut().find(|b| b.address == url) {
            *old = item;
        } else {
            if data.bookmarks.len() >= MAX_BOOKMARKS {
                return Err(invalid("bookmark limit reached (256)"));
            }
            data.bookmarks.push(item);
        }
        data.bookmarks.sort_by(|a, b| {
            (&a.folder, &a.title, &a.address).cmp(&(&b.folder, &b.title, &b.address))
        });
        self.commit(data)
    }
    pub fn remove_bookmark(&mut self, url: &str) -> io::Result<()> {
        let mut d = self.data.clone();
        d.bookmarks.retain(|b| b.address != url);
        self.commit(d)
    }
    pub fn set_homepage(&mut self, url: &str) -> io::Result<()> {
        let mut d = self.data.clone();
        d.home = address(url)?;
        self.commit(d)
    }
    pub fn set_history(&mut self, enabled: bool) -> io::Result<()> {
        let mut d = self.data.clone();
        d.history = enabled;
        self.commit(d)
    }
    pub fn clear_history(&mut self) -> io::Result<()> {
        let mut d = self.data.clone();
        d.visits.clear();
        self.commit(d)
    }
    pub fn record(&mut self, url: &str, title: &str) -> io::Result<()> {
        if !self.data.history {
            return Ok(());
        }
        let url = address(url)?;
        field(title, 160)?;
        if !url.starts_with("yami://") && !url.starts_with("ynps://") && !url.starts_with("ynp://")
        {
            return Ok(());
        }
        let mut d = self.data.clone();
        if d.visits.last().is_some_and(|v| v.address == url) {
            d.visits.pop();
        }
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        d.visits.push(Visit {
            address: url,
            title: title.into(),
            time,
        });
        if d.visits.len() > MAX_HISTORY {
            d.visits.remove(0);
        }
        self.commit(d)
    }
    fn commit(&mut self, data: Data) -> io::Result<()> {
        let mut text = format!(
            "YAMILIBRARY1\nhistory\t{}\nhome\t{}\n",
            u8::from(data.history),
            data.home
        );
        let s = data.settings;
        text.push_str(&format!(
            "settings\t{}\t{}\t{}\n",
            u8::from(s.onboarded),
            u8::from(s.local_only),
            u8::from(s.relays)
        ));
        for b in &data.bookmarks {
            text.push_str(&format!("b\t{}\t{}\t{}\n", b.folder, b.title, b.address));
        }
        for v in &data.visits {
            text.push_str(&format!("h\t{}\t{}\t{}\n", v.time, v.title, v.address));
        }
        if text.len() > MAX_FILE {
            return Err(invalid("library file budget exceeded"));
        }
        let path = self.dir.join("library.tmp");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&path)?;
        let result = file
            .write_all(text.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let result = result.and_then(|()| fs::rename(&path, self.dir.join("library.tsv")));
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result?;
        #[cfg(unix)]
        File::open(&self.dir)?.sync_all()?;
        self.data = data;
        Ok(())
    }
}
fn parse(text: &str) -> io::Result<Data> {
    if text.len() > MAX_FILE || !text.ends_with('\n') {
        return Err(invalid("oversized or incomplete library"));
    }
    let mut lines = text.lines();
    if lines.next() != Some("YAMILIBRARY1") {
        return Err(invalid("unsupported library version"));
    }
    let history = match lines.next() {
        Some("history\t0") => false,
        Some("history\t1") => true,
        _ => return Err(invalid("invalid history preference")),
    };
    let home = address(
        lines
            .next()
            .and_then(|l| l.strip_prefix("home\t"))
            .ok_or_else(|| invalid("missing homepage"))?,
    )?;
    let mut d = Data {
        history,
        home,
        ..Data::default()
    };
    let mut settings_seen = false;
    for line in lines {
        let f: Vec<_> = line.split('\t').collect();
        match f.as_slice() {
            ["settings", onboarded, local, relays] if !settings_seen => {
                let flag = |value: &str| match value {
                    "0" => Ok(false),
                    "1" => Ok(true),
                    _ => Err(invalid("invalid settings flag")),
                };
                d.settings = Settings {
                    onboarded: flag(onboarded)?,
                    local_only: flag(local)?,
                    relays: flag(relays)?,
                };
                settings_seen = true;
            }
            ["b", folder, title, url] if d.bookmarks.len() < MAX_BOOKMARKS => {
                field(folder, 64)?;
                field(title, 160)?;
                let url = address(url)?;
                if url.is_empty() || d.bookmarks.iter().any(|b| b.address == url) {
                    return Err(invalid("duplicate or empty bookmark"));
                }
                d.bookmarks.push(Bookmark {
                    folder: (*folder).into(),
                    title: (*title).into(),
                    address: url,
                });
            }
            ["h", time, title, url] if d.visits.len() < MAX_HISTORY => {
                field(title, 160)?;
                d.visits.push(Visit {
                    time: time
                        .parse()
                        .map_err(|_| invalid("invalid history timestamp"))?,
                    title: (*title).into(),
                    address: address(url)?,
                });
            }
            _ => return Err(invalid("invalid library row or entry limit")),
        }
    }
    Ok(d)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restart_identity_folders_history_and_atomic_failure() {
        let dir = std::env::temp_dir().join(format!("yami-library-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let id = yami_identity::NodeIdentity::generate().unwrap();
        let url = format!("yami://{}/home", id.id());
        let mut l = Library::open(&dir).unwrap();
        assert!(!l.history_enabled());
        assert!(Library::open(&dir).is_err());
        l.record(&url, "Home").unwrap();
        assert!(l.visits().is_empty());
        l.save_bookmark(&url, "My name", "Friends").unwrap();
        l.set_homepage(&url).unwrap();
        l.set_history(true).unwrap();
        for n in 0..520 {
            l.record(&format!("yami://{}/p{n}", id.id()), "Page")
                .unwrap();
        }
        assert_eq!(l.visits().len(), MAX_HISTORY);
        drop(l);
        let mut l = Library::open(&dir).unwrap();
        assert_eq!(l.bookmarks()[0].address, url);
        assert_eq!(l.bookmarks()[0].folder, "Friends");
        assert_eq!(l.homepage(), url);
        l.set_history(false).unwrap();
        l.clear_history().unwrap();
        assert!(l.visits().is_empty());
        fs::create_dir(dir.join("library.tmp")).unwrap();
        assert!(l.remove_bookmark(&url).is_err());
        assert_eq!(l.bookmarks().len(), 1);
        fs::remove_dir(dir.join("library.tmp")).unwrap();
        l.remove_bookmark(&url).unwrap();
        drop(l);
        assert!(Library::open(&dir).unwrap().bookmarks().is_empty());
        fs::write(dir.join("library.tsv"), "broken").unwrap();
        assert!(Library::open(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn malformed_fields_are_inert() {
        for s in ["\t", "\n", "\u{0}"] {
            assert!(field(s, 160).is_err());
        }
        assert!(address("yami://fake/home").is_err());
        assert!(parse("YAMILIBRARY2\n").is_err());
    }
}

#[cfg(test)]
mod alpha_tests {
    use super::*;
    #[test]
    fn legacy_library_gets_safe_defaults_and_invalid_settings_fail() {
        let legacy = "YAMILIBRARY1\nhistory\t0\nhome\t\n";
        assert_eq!(parse(legacy).unwrap().settings, Settings::default());
        assert!(parse(&format!("{legacy}settings\t1\tmaybe\t0\n")).is_err());
        assert!(parse(&format!("{legacy}settings\t1\t0\t0\nsettings\t1\t0\t0\n")).is_err());
    }
    #[test]
    fn preferences_survive_reopen_and_reset_preserves_bookmarks_and_key() {
        let dir = std::env::temp_dir().join(format!("yami-alpha-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("node.key"), b"untouched identity fixture").unwrap();
        let mut library = Library::open(&dir).unwrap();
        library
            .save_bookmark("local:welcome", "Welcome", "")
            .unwrap();
        let settings = Settings {
            onboarded: true,
            local_only: false,
            relays: true,
        };
        library.set_settings(settings).unwrap();
        drop(library);
        let mut library = Library::open(&dir).unwrap();
        assert_eq!(library.settings(), settings);
        library.reset_settings().unwrap();
        assert_eq!(library.settings(), Settings::default());
        assert!(!library.history_enabled());
        assert_eq!(library.bookmarks().len(), 1);
        assert_eq!(
            std::fs::read(dir.join("node.key")).unwrap(),
            b"untouched identity fixture"
        );
        drop(library);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
