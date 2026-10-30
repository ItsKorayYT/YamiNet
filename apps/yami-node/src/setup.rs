//! Guided, offline host preparation built on production authoring and config APIs.
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};
use yami_identity::NodeIdentity;

pub struct Prepared {
    pub config: yami_config::Config,
    pub output: PathBuf,
    pub id: yami_identity::NodeId,
    pub pages: usize,
    pub entry_key: String,
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn absolute(path: &Path) -> io::Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    if path
        .to_str()
        .is_none_or(|s| s.chars().any(char::is_control))
    {
        return Err(invalid("paths must be Unicode without control characters"));
    }
    Ok(path)
}
/// Creates a new config only. Existing keys are loaded and never replaced.
/// No network or listener is used; starting is a separate CLI action.
pub fn prepare(
    site: &Path,
    identity: &Path,
    output: &Path,
    template: Option<&str>,
    relay: Option<&str>,
    bootstraps: &[String],
) -> io::Result<Prepared> {
    let output = absolute(output)?;
    if output.symlink_metadata().is_ok() {
        return Err(invalid(
            "output already exists; inspect it or choose a new config filename",
        ));
    }
    let site = absolute(site)?;
    let identity = absolute(identity)?;
    if identity.starts_with(&site) {
        return Err(invalid("keep identities outside the website directory"));
    }
    if let Some(template) = template {
        if let Some(parent) = site.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::authoring::new_site(&site, template)?;
    }
    let report = crate::authoring::check(&site)?;
    let parent = output
        .parent()
        .ok_or_else(|| invalid("config needs a parent directory"))?;
    fs::create_dir_all(parent)?;
    let state = fs::canonicalize(parent)?
        .join(
            output
                .file_stem()
                .ok_or_else(|| invalid("config filename required"))?,
        )
        .with_extension("state");
    let mode = if relay.is_some() {
        "public"
    } else {
        "loopback"
    };
    let mut text = format!("yaminet-config 1\nrole node\nnetwork {mode}\npublic-listen no\nidentity {}\nstate {}\nsite {}\n", identity.display(), state.display(), fs::canonicalize(&site)?.display());
    if let Some(pin) = relay {
        text.push_str(&format!("relay {pin}\n"));
    } else {
        text.push_str("listen 127.0.0.1:7443\nadvertise 127.0.0.1:7443\ndht yes\n");
    }
    for pin in bootstraps {
        text.push_str(&format!("bootstrap {pin}\n"));
    }
    // Validate all user input before identity creation or config writes.
    let config = yami_config::Config::parse(&text)?;
    let (key, _) = NodeIdentity::load_or_create(&identity).map_err(io::Error::other)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&output)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(Prepared {
        config,
        output,
        id: *key.id(),
        pages: report.pages.len(),
        entry_key: report
            .pages
            .iter()
            .find(|key| key.as_str() == "home")
            .unwrap_or(&report.pages[0])
            .clone(),
    })
}
