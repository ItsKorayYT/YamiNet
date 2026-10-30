//! Opt-in bounded local event codes. No addresses, content, keys or free-text errors.
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub const MAX_LOG: usize = 64 * 1024;
const EVENTS: &[&str] = &[
    "startup",
    "shutdown",
    "navigation-start",
    "navigation-ready",
    "navigation-failed",
    "settings-saved",
    "settings-failed",
    "window-resized",
    "tab-closed",
];
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "diagnostics require bounded regular local files; close the browser before reporting or clearing")
}
fn regular(path: &Path) -> io::Result<()> {
    match path.symlink_metadata() {
        Ok(m) if !m.file_type().is_file() || m.len() > MAX_LOG as u64 => Err(invalid()),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
fn options() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.write(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o
}
pub struct Log {
    dir: PathBuf,
    _lock: File,
}
impl Log {
    pub fn open(dir: &Path) -> io::Result<Self> {
        if !dir.is_absolute() {
            return Err(invalid());
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        if !dir.symlink_metadata()?.file_type().is_dir() {
            return Err(invalid());
        }
        regular(&dir.join("diagnostics.lock"))?;
        let mut o = options();
        o.read(true).truncate(false);
        let lock = o.open(dir.join("diagnostics.lock"))?;
        lock.try_lock().map_err(|_| invalid())?;
        for name in ["events.log", "events.previous.log"] {
            regular(&dir.join(name))?;
        }
        Ok(Self {
            dir: dir.into(),
            _lock: lock,
        })
    }
    pub fn event(&mut self, code: &'static str) -> io::Result<()> {
        if !EVENTS.contains(&code) {
            return Err(invalid());
        }
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_secs();
        let line = format!("{at} {code}\n");
        let current = self.dir.join("events.log");
        let previous = self.dir.join("events.previous.log");
        regular(&current)?;
        regular(&previous)?;
        if current.metadata().map_or(0, |m| m.len()) + line.len() as u64 > MAX_LOG as u64 {
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            fs::rename(&current, &previous)?;
        }
        let mut o = options();
        o.append(true);
        let mut f = o.open(current)?;
        f.write_all(line.as_bytes())?;
        f.sync_all()
    }
    pub fn report(&self) -> io::Result<String> {
        let mut counts = BTreeMap::<String, usize>::new();
        for name in ["events.previous.log", "events.log"] {
            let path = self.dir.join(name);
            regular(&path)?;
            if !path.exists() {
                continue;
            }
            let mut text = String::new();
            File::open(path)?
                .take(MAX_LOG as u64 + 1)
                .read_to_string(&mut text)?;
            if text.len() > MAX_LOG {
                return Err(invalid());
            }
            for line in text.lines() {
                let (at, code) = line.split_once(' ').ok_or_else(invalid)?;
                at.parse::<u64>().map_err(|_| invalid())?;
                if !EVENTS.contains(&code) {
                    return Err(invalid());
                }
                *counts.entry(code.into()).or_default() += 1;
            }
        }
        serde_json::to_string_pretty(&serde_json::json!({"schema":"yaminet-local-diagnostics-1", "version":env!("CARGO_PKG_VERSION"), "os":std::env::consts::OS, "events":counts, "redaction":"event counts only; no addresses, timestamps, paths, content or keys"})).map_err(io::Error::other)
    }
    pub fn clear(&self) -> io::Result<()> {
        for name in ["events.log", "events.previous.log"] {
            let p = self.dir.join(name);
            regular(&p)?;
            if p.exists() {
                fs::remove_file(p)?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_rotate_redact_reject_injected_data_and_clear_only_owned_files() {
        let dir = std::env::temp_dir().join(format!("yami-log-{}", std::process::id()));
        let mut log = Log::open(&dir).unwrap();
        assert!(Log::open(&dir).is_err());
        for _ in 0..4000 {
            log.event("navigation-ready").unwrap();
        }
        for name in ["events.log", "events.previous.log"] {
            assert!(dir.join(name).metadata().unwrap().len() <= MAX_LOG as u64);
        }
        assert!(log.report().unwrap().contains("navigation-ready"));
        fs::write(dir.join("node.key"), b"secret fixture").unwrap();
        log.clear().unwrap();
        assert!(dir.join("node.key").exists());
        fs::write(dir.join("events.log"), b"123 yami://private/address\n").unwrap();
        assert!(log.report().is_err());
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn log_rejects_symlinks_and_creates_owner_only_files() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("yami-log-symlink-{}", std::process::id()));
        let mut log = Log::open(&dir).unwrap();
        log.event("startup").unwrap();
        assert_eq!(
            dir.join("events.log")
                .metadata()
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_file(dir.join("events.log")).unwrap();
        symlink("node.key", dir.join("events.log")).unwrap();
        assert!(log.event("shutdown").is_err());
        drop(log);
        fs::remove_dir_all(dir).unwrap();
    }
}
