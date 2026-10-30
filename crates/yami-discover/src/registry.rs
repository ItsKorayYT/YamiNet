//! Persistent local source registry and bounded cached catalog snapshots.
//!
//! The registry stores only local source settings: a unique display name and a
//! canonical local path or an inactive `yami://` subscription address. It never
//! stores keys, credentials, query text or trust state. Remote subscriptions are
//! recorded for future compatibility but are never fetched or marked current.
//!
//! Snapshot files are ordinary version-1 directory documents. They are bounded
//! by the same parser and limits as every other directory, and are used only as
//! an explicit fallback when a local source is unavailable at rebuild time.
use crate::{
    catalog::source_label,
    store::{self, Mode},
    Catalog, Directory, Error, Result, SearchHit, WebsiteAddress, MAX_REGISTRY_BYTES, MAX_SOURCES,
    MAX_SOURCE_PATH_BYTES, MAX_STATUS_BYTES, MAX_STATUS_REASON_CHARS,
};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

const REGISTRY_HEADER: &str = "yami-discover-sources 1";
const STATUS_HEADER: &str = "yami-discover-source-status 1";
const SOURCES_FILE: &str = "sources.conf";
const SNAPSHOTS_DIRECTORY: &str = "snapshots";

/// A registered source's location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind {
    /// A local version-1 directory document.
    Local(PathBuf),
    /// An inactive future `yami://` subscription. Never fetched by this build.
    Remote(WebsiteAddress),
}

/// One registered directory source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    name: String,
    kind: SourceKind,
}

impl Source {
    /// The unique importer-facing source name, also used as the catalog label.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether this is a local file or an inactive remote subscription.
    pub fn kind(&self) -> &SourceKind {
        &self.kind
    }

    /// The local directory path, when this is a local source.
    pub fn local_path(&self) -> Option<&Path> {
        match &self.kind {
            SourceKind::Local(path) => Some(path),
            SourceKind::Remote(_) => None,
        }
    }

    /// The inactive remote subscription address, when present.
    pub fn remote_address(&self) -> Option<&WebsiteAddress> {
        match &self.kind {
            SourceKind::Local(_) => None,
            SourceKind::Remote(address) => Some(address),
        }
    }
}

/// Provenance of the listings retained for one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceState {
    /// The source file was loaded successfully at the last explicit refresh.
    Fresh,
    /// The source file was unavailable, so its validated snapshot was used.
    Cached { reason: String },
    /// Neither the source nor a usable snapshot was available.
    Unavailable { reason: String },
    /// A remote subscription. It is deliberately inactive and never fetched.
    InactiveRemote,
    /// The source was registered but no catalog snapshot has been built yet.
    NotBuilt,
}

impl SourceState {
    /// Whether the listings came from a freshly loaded source.
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Fresh)
    }

    /// A short stable label suitable for CLI and browser display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Cached { .. } => "cached",
            Self::Unavailable { .. } => "unavailable",
            Self::InactiveRemote => "inactive remote",
            Self::NotBuilt => "not built",
        }
    }

    /// A bounded explanation for cached, unavailable or corrupt states.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Cached { reason } | Self::Unavailable { reason } => Some(reason),
            _ => None,
        }
    }
}

/// Bounded status for one registered source in a catalog view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    pub name: String,
    pub state: SourceState,
    /// Unique entries contributed after first-source-wins deduplication.
    pub entries: usize,
}

/// A merged catalog plus per-source provenance.
///
/// Search delegates to the existing [`Catalog::search`] implementation. Listed
/// websites remain untrusted hints; provenance describes local file/cache use,
/// never identity trust.
#[derive(Debug)]
pub struct CatalogView {
    catalog: Catalog,
    statuses: Vec<SourceStatus>,
}

impl CatalogView {
    /// The merged catalog, for callers that need the full [`Catalog`] API.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Statuses in deterministic source-registry order.
    pub fn statuses(&self) -> &[SourceStatus] {
        &self.statuses
    }

    /// Finds the status for one source name.
    pub fn status_for(&self, name: &str) -> Option<&SourceStatus> {
        self.statuses.iter().find(|status| status.name == name)
    }

    /// Searches the merged catalog with the existing local Unicode search.
    pub fn search(&self, query: &str) -> Result<Vec<SearchHit<'_>>> {
        self.catalog.search(query)
    }

    /// The merged entries in deterministic import order.
    pub fn entries(&self) -> &[crate::Entry] {
        self.catalog.entries()
    }

    /// Number of unique entries in this view.
    pub fn len(&self) -> usize {
        self.catalog.len()
    }

    /// Whether this view contains no entries.
    pub fn is_empty(&self) -> bool {
        self.catalog.is_empty()
    }
}

/// Versioned persistent list of directory sources.
///
/// Source order is the order of registration and is preserved across restarts.
/// Names are unique. The registry never contains keys or authentication data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceRegistry {
    sources: Vec<Source>,
}

impl SourceRegistry {
    /// Loads `sources.conf` from a state directory. A missing file is an empty
    /// registry; a malformed or oversized file is an error.
    pub fn load(state_directory: impl AsRef<Path>) -> Result<Self> {
        let path = registry_path(state_directory.as_ref());
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_file() {
            return Err(Error::registry(
                "sources.conf must be a regular, non-symlink file",
            ));
        }
        if metadata.len() > MAX_REGISTRY_BYTES as u64 {
            return Err(Error::TooLarge {
                limit: MAX_REGISTRY_BYTES,
            });
        }
        let mut source = String::new();
        File::open(&path)?
            .take((MAX_REGISTRY_BYTES + 1) as u64)
            .read_to_string(&mut source)?;
        Self::parse(&source)
    }

    /// Parses a versioned registry document.
    pub fn parse(source: &str) -> Result<Self> {
        if source.len() > MAX_REGISTRY_BYTES {
            return Err(Error::TooLarge {
                limit: MAX_REGISTRY_BYTES,
            });
        }
        let mut lines = source.lines().filter(|line| !line.trim().is_empty());
        let header = lines
            .next()
            .ok_or_else(|| Error::registry("missing header"))?;
        if header.trim() != REGISTRY_HEADER {
            return Err(Error::registry(format!(
                "expected header {REGISTRY_HEADER:?}"
            )));
        }
        let mut sources = Vec::new();
        let mut names = HashSet::new();
        for (index, line) in lines.enumerate() {
            let line_number = index + 2;
            let fields: Vec<&str> = line.split_whitespace().collect();
            let (kind, name_hex, target_hex) = match fields.as_slice() {
                ["local", name, path] => ("local", *name, *path),
                ["remote", name, address] => ("remote", *name, *address),
                _ => {
                    return Err(Error::registry(format!(
                        "line {line_number}: expected local NAME PATH or remote NAME ADDRESS"
                    )))
                }
            };
            let name = decode_text(name_hex)
                .map_err(|message| Error::registry(format!("line {line_number}: {message}")))?;
            let name = source_label(&name)
                .map_err(|error| Error::registry(format!("line {line_number}: {error}")))?;
            if sources.len() >= MAX_SOURCES {
                return Err(Error::limit(format!(
                    "at most {MAX_SOURCES} directory sources can be registered"
                )));
            }
            if !names.insert(name.clone()) {
                return Err(Error::Conflict {
                    message: format!("source {name:?} is registered more than once"),
                });
            }
            let target = decode_text(target_hex)
                .map_err(|message| Error::registry(format!("line {line_number}: {message}")))?;
            let kind = if kind == "local" {
                SourceKind::Local(
                    validate_stored_path(&target)
                        .map_err(|error| Error::registry(format!("line {line_number}: {error}")))?,
                )
            } else {
                SourceKind::Remote(WebsiteAddress::parse(&target).map_err(|error| {
                    Error::registry(format!(
                        "line {line_number}: inactive remote address: {error}"
                    ))
                })?)
            };
            sources.push(Source { name, kind });
        }
        Ok(Self { sources })
    }

    /// Renders the versioned registry document.
    pub fn render(&self) -> Result<String> {
        let mut source = String::from(REGISTRY_HEADER);
        source.push('\n');
        for entry in &self.sources {
            match &entry.kind {
                SourceKind::Local(path) => {
                    let path = path
                        .to_str()
                        .ok_or_else(|| Error::registry("local source paths must be UTF-8"))?;
                    source.push_str("local ");
                    source.push_str(&hex(entry.name.as_bytes()));
                    source.push(' ');
                    source.push_str(&hex(path.as_bytes()));
                    source.push('\n');
                }
                SourceKind::Remote(address) => {
                    source.push_str("remote ");
                    source.push_str(&hex(entry.name.as_bytes()));
                    source.push(' ');
                    source.push_str(&hex(address.to_string().as_bytes()));
                    source.push('\n');
                }
            }
        }
        if source.len() > MAX_REGISTRY_BYTES {
            return Err(Error::TooLarge {
                limit: MAX_REGISTRY_BYTES,
            });
        }
        Ok(source)
    }

    /// Saves the registry atomically under the state directory.
    pub fn save(&self, state_directory: impl AsRef<Path>) -> Result<()> {
        let state_directory = state_directory.as_ref();
        let source = self.render()?;
        fs::create_dir_all(state_directory)?;
        store::write_atomic(
            &registry_path(state_directory),
            &source,
            Mode::ReplaceOrCreate,
        )
    }

    /// Registers a local directory file after validating it with the existing
    /// parser. Relative paths are made absolute before storage.
    pub fn register_local(&mut self, name: &str, path: impl AsRef<Path>) -> Result<Source> {
        let name = source_label(name)?;
        ensure_source_capacity(self.sources.len())?;
        if self.get(&name).is_some() {
            return Err(Error::Conflict {
                message: format!("source {name:?} is already registered"),
            });
        }
        let path = absolute_source_path(path.as_ref())?;
        Directory::load(&path).map_err(|error| {
            Error::source(format!("cannot register {}: {error}", path.display()))
        })?;
        let source = Source {
            name,
            kind: SourceKind::Local(path),
        };
        self.sources.push(source.clone());
        Ok(source)
    }

    /// Registers an inactive remote `yami://` directory subscription.
    ///
    /// This build performs no network access and never marks the subscription
    /// as fetched. Its status remains [`SourceState::InactiveRemote`].
    pub fn register_remote(&mut self, name: &str, address: WebsiteAddress) -> Result<Source> {
        let name = source_label(name)?;
        ensure_source_capacity(self.sources.len())?;
        if self.get(&name).is_some() {
            return Err(Error::Conflict {
                message: format!("source {name:?} is already registered"),
            });
        }
        let source = Source {
            name,
            kind: SourceKind::Remote(address),
        };
        self.sources.push(source.clone());
        Ok(source)
    }

    /// Removes a source by exact name.
    pub fn remove(&mut self, name: &str) -> Result<Source> {
        let name = name.trim();
        let index = self
            .sources
            .iter()
            .position(|source| source.name == name)
            .ok_or_else(|| Error::source(format!("no source named {name:?}")))?;
        Ok(self.sources.remove(index))
    }

    /// Looks up a source by exact name.
    pub fn get(&self, name: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.name == name)
    }

    /// Sources in deterministic registration order.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Number of registered sources.
    pub fn len(&self) -> usize {
        self.sources.len()
    }

    /// Whether no sources are registered.
    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

/// Persistent state directory handle for a local discovery catalog.
///
/// The state directory contains `sources.conf`, `snapshots/*.ylang` and
/// `snapshots/*.status`. All writes are atomic. No operation reads a source
/// unless it is an explicit [`Self::rebuild`] or [`Self::refresh_source`] call.
#[derive(Debug, Clone)]
pub struct DiscoveryStore {
    state_directory: PathBuf,
    registry: SourceRegistry,
}

impl DiscoveryStore {
    /// Opens a store, loading only the source registry. Missing state is empty.
    pub fn open(state_directory: impl Into<PathBuf>) -> Result<Self> {
        let state_directory = state_directory.into();
        let registry = SourceRegistry::load(&state_directory)?;
        Ok(Self {
            state_directory,
            registry,
        })
    }

    /// The state directory used for the registry and snapshots.
    pub fn state_directory(&self) -> &Path {
        &self.state_directory
    }

    /// The current registry in deterministic order.
    pub fn registry(&self) -> &SourceRegistry {
        &self.registry
    }

    /// Registers and validates a local source, then persists the registry.
    pub fn register_local(&mut self, name: &str, path: impl AsRef<Path>) -> Result<Source> {
        let mut next = self.registry.clone();
        let source = next.register_local(name, path)?;
        next.save(&self.state_directory)?;
        self.registry = next;
        Ok(source)
    }

    /// Registers an inactive remote source, then persists the registry.
    pub fn register_remote(&mut self, name: &str, address: WebsiteAddress) -> Result<Source> {
        let mut next = self.registry.clone();
        let source = next.register_remote(name, address)?;
        next.save(&self.state_directory)?;
        self.registry = next;
        Ok(source)
    }

    /// Removes a source and its cached snapshot/status atomically with respect
    /// to the registry file.
    pub fn remove_source(&mut self, name: &str) -> Result<Source> {
        let mut next = self.registry.clone();
        let removed = next.remove(name)?;
        self.remove_cached_source(&removed.name)?;
        next.save(&self.state_directory)?;
        self.registry = next;
        Ok(removed)
    }

    /// Refreshes every registered local source, rebuilding the catalog.
    ///
    /// Freshly loaded sources are snapshotted. Failed local sources fall back
    /// to their last validated snapshot. Remote subscriptions remain inactive.
    pub fn rebuild(&self) -> Result<CatalogView> {
        let mut catalog = Catalog::new();
        let mut statuses = Vec::with_capacity(self.registry.len());
        for source in self.registry.sources() {
            let (state, entries) = self.rebuild_source(source, &mut catalog)?;
            statuses.push(SourceStatus {
                name: source.name.clone(),
                state,
                entries,
            });
        }
        Ok(CatalogView { catalog, statuses })
    }

    /// Explicitly refreshes one source and reloads the combined catalog from
    /// snapshots. Other source files are not read or refreshed.
    pub fn refresh_source(&self, name: &str) -> Result<CatalogView> {
        let source = self
            .registry
            .get(name.trim())
            .ok_or_else(|| Error::source(format!("no source named {name:?}")))?;
        match source.kind() {
            SourceKind::Local(path) => match Directory::load(path) {
                Ok(directory) => {
                    self.write_snapshot(&source.name, &directory)?;
                    self.write_status(&source.name, &SourceState::Fresh)?;
                }
                Err(source_error) => {
                    let source_reason = reason_text(source_error.to_string());
                    let state = match self.read_snapshot(&source.name) {
                        Ok(Some(_)) => SourceState::Cached {
                            reason: format!("source unavailable: {source_reason}"),
                        },
                        Ok(None) => SourceState::Unavailable {
                            reason: format!("source unavailable: {source_reason}; no snapshot"),
                        },
                        Err(snapshot_error) => SourceState::Unavailable {
                            reason: format!(
                                "source unavailable: {source_reason}; snapshot invalid: {snapshot_error}"
                            ),
                        },
                    };
                    self.write_status(&source.name, &state)?;
                }
            },
            SourceKind::Remote(_) => {
                // Explicitly inactive: no network request and no fetched mark.
                self.write_status(&source.name, &SourceState::InactiveRemote)?;
            }
        }
        self.load_last()
    }

    /// Loads the combined catalog from cached snapshots without reading any
    /// source files. This is the no-automatic-refresh search path.
    pub fn load_last(&self) -> Result<CatalogView> {
        let mut catalog = Catalog::new();
        let mut statuses = Vec::with_capacity(self.registry.len());
        for source in self.registry.sources() {
            let status = self.cached_status(source, &mut catalog);
            statuses.push(status);
        }
        Ok(CatalogView { catalog, statuses })
    }

    fn rebuild_source(
        &self,
        source: &Source,
        catalog: &mut Catalog,
    ) -> Result<(SourceState, usize)> {
        match source.kind() {
            SourceKind::Remote(_) => {
                let state = SourceState::InactiveRemote;
                self.write_status(&source.name, &state)?;
                Ok((state, 0))
            }
            SourceKind::Local(path) => match Directory::load(path) {
                Ok(directory) => match catalog.add_directory(&source.name, directory.clone()) {
                    Ok(report) => {
                        self.write_snapshot(&source.name, &directory)?;
                        self.write_status(&source.name, &SourceState::Fresh)?;
                        Ok((SourceState::Fresh, report.source.added))
                    }
                    Err(error) => {
                        let state = SourceState::Unavailable {
                            reason: format!("catalog rejected source: {error}"),
                        };
                        self.write_status(&source.name, &state)?;
                        Ok((state, 0))
                    }
                },
                Err(source_error) => {
                    let source_reason = reason_text(source_error.to_string());
                    match self.read_snapshot(&source.name) {
                        Ok(Some(snapshot)) => match catalog.add_directory(&source.name, snapshot) {
                            Ok(report) => {
                                let state = SourceState::Cached {
                                    reason: format!("source unavailable: {source_reason}"),
                                };
                                self.write_status(&source.name, &state)?;
                                Ok((state, report.source.added))
                            }
                            Err(error) => {
                                let state = SourceState::Unavailable {
                                    reason: format!("cached entries rejected: {error}"),
                                };
                                self.write_status(&source.name, &state)?;
                                Ok((state, 0))
                            }
                        },
                        Ok(None) => {
                            let state = SourceState::Unavailable {
                                reason: format!("source unavailable: {source_reason}; no snapshot"),
                            };
                            self.write_status(&source.name, &state)?;
                            Ok((state, 0))
                        }
                        Err(snapshot_error) => {
                            let state = SourceState::Unavailable {
                                reason: format!(
                                    "source unavailable: {source_reason}; snapshot invalid: {snapshot_error}"
                                ),
                            };
                            self.write_status(&source.name, &state)?;
                            Ok((state, 0))
                        }
                    }
                }
            },
        }
    }

    fn cached_status(&self, source: &Source, catalog: &mut Catalog) -> SourceStatus {
        if matches!(source.kind(), SourceKind::Remote(_)) {
            return SourceStatus {
                name: source.name.clone(),
                state: SourceState::InactiveRemote,
                entries: 0,
            };
        }
        let status = self.read_status(&source.name);
        let snapshot = self.read_snapshot(&source.name);
        match snapshot {
            Ok(Some(directory)) => {
                let state = match status {
                    Ok(Some(SourceState::Fresh)) => SourceState::Fresh,
                    Ok(Some(SourceState::Cached { reason })) => SourceState::Cached { reason },
                    Ok(Some(SourceState::Unavailable { reason })) => SourceState::Cached {
                        reason: format!("using retained snapshot after: {reason}"),
                    },
                    Ok(Some(SourceState::InactiveRemote)) => SourceState::Cached {
                        reason: "invalid remote status on a local source".into(),
                    },
                    Ok(Some(SourceState::NotBuilt)) => SourceState::Cached {
                        reason: "status says not built; using snapshot".into(),
                    },
                    Ok(None) => SourceState::Cached {
                        reason: "no build status; using cached snapshot".into(),
                    },
                    Err(error) => SourceState::Cached {
                        reason: format!("corrupt status; using cached snapshot: {error}"),
                    },
                };
                match catalog.add_directory(&source.name, directory) {
                    Ok(report) => SourceStatus {
                        name: source.name.clone(),
                        state,
                        entries: report.source.added,
                    },
                    Err(error) => SourceStatus {
                        name: source.name.clone(),
                        state: SourceState::Unavailable {
                            reason: format!("catalog rejected cached entries: {error}"),
                        },
                        entries: 0,
                    },
                }
            }
            Ok(None) => {
                let state = match status {
                    Ok(Some(SourceState::Unavailable { reason })) => {
                        SourceState::Unavailable { reason }
                    }
                    Ok(Some(SourceState::Fresh | SourceState::Cached { .. })) => {
                        SourceState::Unavailable {
                            reason: "snapshot is missing; run catalog rebuild".into(),
                        }
                    }
                    Ok(Some(SourceState::InactiveRemote)) => SourceState::Unavailable {
                        reason: "invalid remote status on a local source".into(),
                    },
                    Ok(Some(SourceState::NotBuilt)) | Ok(None) => SourceState::NotBuilt,
                    Err(error) => SourceState::Unavailable {
                        reason: format!("corrupt status and no snapshot: {error}"),
                    },
                };
                SourceStatus {
                    name: source.name.clone(),
                    state,
                    entries: 0,
                }
            }
            Err(error) => SourceStatus {
                name: source.name.clone(),
                state: SourceState::Unavailable {
                    reason: format!("snapshot invalid: {error}"),
                },
                entries: 0,
            },
        }
    }

    fn write_snapshot(&self, name: &str, directory: &Directory) -> Result<()> {
        let source = directory.render()?;
        self.ensure_snapshots_directory()?;
        store::write_atomic(&self.snapshot_path(name), &source, Mode::ReplaceOrCreate)
    }

    fn read_snapshot(&self, name: &str) -> std::result::Result<Option<Directory>, String> {
        let path = self.snapshot_path(name);
        match fs::symlink_metadata(&path) {
            Ok(_) => Directory::load(&path)
                .map(Some)
                .map_err(|error| error.to_string()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    fn write_status(&self, name: &str, state: &SourceState) -> Result<()> {
        let source = encode_status(state);
        self.ensure_snapshots_directory()?;
        store::write_atomic(&self.status_path(name), &source, Mode::ReplaceOrCreate)
    }

    fn read_status(&self, name: &str) -> std::result::Result<Option<SourceState>, String> {
        let path = self.status_path(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.file_type().is_file() {
            return Err("status is not a regular, non-symlink file".into());
        }
        if metadata.len() > MAX_STATUS_BYTES as u64 {
            return Err("status file exceeds its byte limit".into());
        }
        let mut source = String::new();
        File::open(&path)
            .map_err(|error| error.to_string())?
            .take((MAX_STATUS_BYTES + 1) as u64)
            .read_to_string(&mut source)
            .map_err(|error| error.to_string())?;
        parse_status(&source)
    }

    fn ensure_snapshots_directory(&self) -> Result<()> {
        fs::create_dir_all(self.state_directory.join(SNAPSHOTS_DIRECTORY))?;
        Ok(())
    }

    fn remove_cached_source(&self, name: &str) -> Result<()> {
        for path in [self.snapshot_path(name), self.status_path(name)] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn snapshot_path(&self, name: &str) -> PathBuf {
        self.state_directory
            .join(SNAPSHOTS_DIRECTORY)
            .join(format!("{}.ylang", source_id(name)))
    }

    fn status_path(&self, name: &str) -> PathBuf {
        self.state_directory
            .join(SNAPSHOTS_DIRECTORY)
            .join(format!("{}.status", source_id(name)))
    }
}

fn registry_path(state_directory: &Path) -> PathBuf {
    state_directory.join(SOURCES_FILE)
}

fn ensure_source_capacity(current: usize) -> Result<()> {
    if current >= MAX_SOURCES {
        Err(Error::limit(format!(
            "at most {MAX_SOURCES} directory sources can be registered"
        )))
    } else {
        Ok(())
    }
}

fn absolute_source_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path)
        .map_err(|error| Error::source(format!("invalid local source path: {error}")))?;
    if !absolute.is_absolute() {
        return Err(Error::source("local source path must be absolute"));
    }
    let text = absolute
        .to_str()
        .ok_or_else(|| Error::source("local source paths must be UTF-8"))?;
    if text.len() > MAX_SOURCE_PATH_BYTES {
        return Err(Error::source(format!(
            "local source path exceeds {MAX_SOURCE_PATH_BYTES} bytes"
        )));
    }
    Ok(absolute)
}

fn validate_stored_path(text: &str) -> std::result::Result<PathBuf, String> {
    if text.is_empty() {
        return Err("local source path cannot be empty".into());
    }
    if text.len() > MAX_SOURCE_PATH_BYTES {
        return Err(format!(
            "local source path exceeds {MAX_SOURCE_PATH_BYTES} bytes"
        ));
    }
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err("local source path must be absolute".into());
    }
    Ok(path)
}

fn source_id(name: &str) -> String {
    hex(&yami_identity::sha256(name.as_bytes()))
}

fn reason_text(text: impl Into<String>) -> String {
    let text: String = text
        .into()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let mut text: String = text.trim().chars().take(MAX_STATUS_REASON_CHARS).collect();
    if text.is_empty() {
        text = "unknown error".into();
    }
    text
}

fn encode_status(state: &SourceState) -> String {
    let body = match state {
        SourceState::Fresh => "fresh".to_string(),
        SourceState::Cached { reason } => {
            format!("cached {}", hex(reason_text(reason).as_bytes()))
        }
        SourceState::Unavailable { reason } => {
            format!("unavailable {}", hex(reason_text(reason).as_bytes()))
        }
        SourceState::InactiveRemote => "inactive-remote".to_string(),
        SourceState::NotBuilt => "not-built".to_string(),
    };
    format!("{STATUS_HEADER}\n{body}\n")
}

fn parse_status(source: &str) -> std::result::Result<Option<SourceState>, String> {
    if source.len() > MAX_STATUS_BYTES {
        return Err("status file exceeds its byte limit".into());
    }
    let mut lines = source.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().ok_or("missing status header")?;
    if header.trim() != STATUS_HEADER {
        return Err(format!("expected status header {STATUS_HEADER:?}"));
    }
    let line = lines.next().ok_or("missing status state")?;
    if lines.next().is_some() {
        return Err("unexpected extra status lines".into());
    }
    let fields: Vec<&str> = line.split_whitespace().collect();
    let state = match fields.as_slice() {
        ["fresh"] => SourceState::Fresh,
        ["cached", value] => SourceState::Cached {
            reason: decode_reason(value)?,
        },
        ["unavailable", value] => SourceState::Unavailable {
            reason: decode_reason(value)?,
        },
        ["inactive-remote"] => SourceState::InactiveRemote,
        ["not-built"] => SourceState::NotBuilt,
        _ => return Err("invalid status state".into()),
    };
    Ok(Some(state))
}

fn decode_reason(value: &str) -> std::result::Result<String, String> {
    let bytes = unhex(value).ok_or("status reason is not canonical lowercase hex")?;
    let text = String::from_utf8(bytes).map_err(|_| "status reason is not UTF-8")?;
    Ok(reason_text(text))
}

fn decode_text(value: &str) -> std::result::Result<String, String> {
    let bytes = unhex(value).ok_or("field is not canonical lowercase hex")?;
    String::from_utf8(bytes).map_err(|_| "field is not UTF-8".into())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    fn digit(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        output.push((digit(pair[0])? << 4) | digit(pair[1])?);
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DirectoryEntry, MAX_REGISTRY_BYTES, MAX_SOURCE_LABEL_CHARS, MAX_TOTAL_ENTRIES};

    const ID: &str = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re";

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "yami-discover-registry-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn make_source(path: &Path, name: &str, entries: &[(String, String)]) {
        let mut directory = Directory::new(name, vec![]).unwrap();
        for (title, key) in entries {
            let address = WebsiteAddress::parse(&format!("yami://{ID}/{key}")).unwrap();
            directory
                .add_entry(
                    DirectoryEntry::new(title, format!("Description for {title}"), address, None)
                        .unwrap(),
                )
                .unwrap();
        }
        directory.save_new(path).unwrap();
    }

    fn simple_source(path: &Path, name: &str, title: &str, key: &str) {
        make_source(path, name, &[(title.to_string(), key.to_string())]);
    }

    #[test]
    fn registry_round_trips_and_catalog_survives_restart() {
        let root = temp_root("restart");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        let first = sources.join("first.ylang");
        let second = sources.join("second.ylang");
        simple_source(&first, "First", "Café Notes", "home");
        make_source(
            &second,
            "Second",
            &[
                ("日本語の手帖".into(), "about".into()),
                ("Another".into(), "notes".into()),
            ],
        );

        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("zeta", &first).unwrap();
        store.register_local("café 日本語", &second).unwrap();
        let registry = store.registry().clone();
        drop(store);

        let reopened = DiscoveryStore::open(&state).unwrap();
        assert_eq!(reopened.registry(), &registry);
        let names: Vec<_> = reopened
            .registry()
            .sources()
            .iter()
            .map(Source::name)
            .collect();
        assert_eq!(names, ["zeta", "café 日本語"]);
        assert!(reopened.registry().sources()[0]
            .local_path()
            .unwrap()
            .is_absolute());

        let view = reopened.rebuild().unwrap();
        assert_eq!(view.len(), 3);
        assert_eq!(
            view.statuses()
                .iter()
                .map(|status| status.state.label())
                .collect::<Vec<_>>(),
            ["fresh", "fresh"]
        );
        assert_eq!(view.status_for("zeta").unwrap().entries, 1);
        assert_eq!(view.search("cafe").unwrap().len(), 1);
        assert_eq!(view.search("日本").unwrap().len(), 1);

        let cached = DiscoveryStore::open(&state).unwrap().load_last().unwrap();
        assert_eq!(cached.len(), 3);
        assert_eq!(cached.search("Café").unwrap().len(), 1);
        let config = fs::read_to_string(state.join("sources.conf")).unwrap();
        assert!(config.starts_with("yami-discover-sources 1\n"));
        assert_eq!(config.lines().count(), 3);
        assert!(state.join("sources.conf").is_file());
        assert_eq!(
            cached
                .statuses()
                .iter()
                .map(|status| status.name.clone())
                .collect::<Vec<_>>(),
            ["zeta", "café 日本語"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn duplicate_addresses_keep_the_first_registered_source() {
        let root = temp_root("duplicates");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        let first = sources.join("first.ylang");
        let second = sources.join("second.ylang");
        simple_source(&first, "First", "First title", "home");
        simple_source(&second, "Second", "Second title", "home");

        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("first", &first).unwrap();
        store.register_local("second", &second).unwrap();
        let view = store.rebuild().unwrap();
        assert_eq!(view.len(), 1);
        assert_eq!(view.entries()[0].title(), "First title");
        assert_eq!(view.entries()[0].source(), "first");
        assert_eq!(view.status_for("second").unwrap().entries, 0);

        let cached = store.load_last().unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached.entries()[0].source(), "first");
        assert_eq!(
            cached
                .statuses()
                .iter()
                .map(|status| status.name.clone())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_source_falls_back_to_its_bounded_snapshot() {
        let root = temp_root("stale");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        let source = sources.join("community.ylang");
        simple_source(&source, "Community", "Cached title", "home");

        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("community", &source).unwrap();
        store.rebuild().unwrap();
        let snapshot = store.snapshot_path("community");
        let status = store.status_path("community");
        assert!(snapshot.is_file());
        assert!(status.is_file());

        fs::remove_file(&source).unwrap();
        // Search does not read source files, so the last snapshot remains.
        let before_refresh = store.load_last().unwrap();
        assert_eq!(before_refresh.len(), 1);
        assert!(before_refresh
            .status_for("community")
            .unwrap()
            .state
            .is_fresh());

        let rebuilt = store.rebuild().unwrap();
        assert_eq!(rebuilt.len(), 1);
        assert!(matches!(
            rebuilt.status_for("community").unwrap().state,
            SourceState::Cached { .. }
        ));
        assert_eq!(rebuilt.search("cached").unwrap().len(), 1);

        fs::remove_file(&snapshot).unwrap();
        let unavailable = store.rebuild().unwrap();
        assert!(unavailable.is_empty());
        assert!(matches!(
            unavailable.status_for("community").unwrap().state,
            SourceState::Unavailable { .. }
        ));
        assert!(DiscoveryStore::open(&state)
            .unwrap()
            .load_last()
            .unwrap()
            .is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupted_registry_snapshot_and_status_are_bounded() {
        let root = temp_root("corrupt");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        fs::create_dir_all(&state).unwrap();
        fs::write(state.join("sources.conf"), "not a registry").unwrap();
        assert!(DiscoveryStore::open(&state).is_err());

        let state = root.join("state2");
        let source = sources.join("community.ylang");
        simple_source(&source, "Community", "Good title", "home");
        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("community", &source).unwrap();
        store.rebuild().unwrap();

        fs::write(store.status_path("community"), "not a status").unwrap();
        let corrupt_status = store.load_last().unwrap();
        assert_eq!(corrupt_status.len(), 1);
        assert!(matches!(
            corrupt_status.status_for("community").unwrap().state,
            SourceState::Cached { .. }
        ));

        fs::write(store.snapshot_path("community"), "not ylang").unwrap();
        fs::remove_file(&source).unwrap();
        let rebuilt = store.rebuild().unwrap();
        assert!(rebuilt.is_empty());
        assert!(matches!(
            rebuilt.status_for("community").unwrap().state,
            SourceState::Unavailable { .. }
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn source_removal_deletes_registry_entry_and_cache() {
        let root = temp_root("remove");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        let first = sources.join("first.ylang");
        let second = sources.join("second.ylang");
        simple_source(&first, "First", "First", "home");
        simple_source(&second, "Second", "Second", "about");

        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("first", &first).unwrap();
        store.register_local("second", &second).unwrap();
        store.rebuild().unwrap();
        let first_snapshot = store.snapshot_path("first");
        let first_status = store.status_path("first");
        assert!(first_snapshot.is_file());
        assert!(first_status.is_file());

        store.remove_source("first").unwrap();
        assert!(!first_snapshot.exists());
        assert!(!first_status.exists());
        assert_eq!(store.registry().len(), 1);
        assert_eq!(store.registry().sources()[0].name(), "second");
        assert!(store.remove_source("missing").is_err());
        assert_eq!(store.load_last().unwrap().len(), 1);
        assert_eq!(
            DiscoveryStore::open(&state)
                .unwrap()
                .registry()
                .sources()
                .len(),
            1
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn individual_refresh_does_not_read_other_sources() {
        let root = temp_root("refresh");
        let sources = root.join("sources");
        let state = root.join("state");
        fs::create_dir_all(&sources).unwrap();
        let first = sources.join("first.ylang");
        let second = sources.join("second.ylang");
        simple_source(&first, "First", "First", "home");
        simple_source(&second, "Second", "Second", "about");

        let mut store = DiscoveryStore::open(&state).unwrap();
        store.register_local("first", &first).unwrap();
        store.register_local("second", &second).unwrap();
        store.rebuild().unwrap();

        let mut updated = Directory::load(&first).unwrap();
        updated
            .add_entry(
                DirectoryEntry::new(
                    "First extra",
                    "Extra",
                    WebsiteAddress::parse(&format!("yami://{ID}/extra")).unwrap(),
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        updated.save(&first).unwrap();

        assert_eq!(store.load_last().unwrap().len(), 2);
        let refreshed = store.refresh_source("first").unwrap();
        assert_eq!(refreshed.len(), 3);
        assert_eq!(refreshed.status_for("first").unwrap().entries, 2);
        assert_eq!(refreshed.status_for("second").unwrap().entries, 1);

        fs::remove_file(&second).unwrap();
        let refreshed_again = store.refresh_source("first").unwrap();
        assert_eq!(refreshed_again.len(), 3);
        assert!(refreshed_again
            .status_for("second")
            .unwrap()
            .state
            .is_fresh());

        let rebuilt = store.rebuild().unwrap();
        assert!(matches!(
            rebuilt.status_for("second").unwrap().state,
            SourceState::Cached { .. }
        ));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn remote_sources_remain_inactive_and_never_get_snapshots() {
        let root = temp_root("remote");
        let state = root.join("state");
        let mut store = DiscoveryStore::open(&state).unwrap();
        let address = WebsiteAddress::parse(&format!("yami://{ID}/home")).unwrap();
        store.register_remote("remote", address).unwrap();
        let view = store.rebuild().unwrap();
        assert!(view.is_empty());
        assert_eq!(
            view.status_for("remote").unwrap().state,
            SourceState::InactiveRemote
        );
        assert!(!store.snapshot_path("remote").exists());

        let refreshed = store.refresh_source("remote").unwrap();
        assert_eq!(
            refreshed.status_for("remote").unwrap().state,
            SourceState::InactiveRemote
        );
        assert!(!store.snapshot_path("remote").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_bounds_and_total_catalog_budget_are_enforced() {
        let root = temp_root("bounds");
        let state = root.join("state");
        let address = WebsiteAddress::parse(&format!("yami://{ID}/home")).unwrap();
        let mut store = DiscoveryStore::open(&state).unwrap();
        for index in 0..MAX_SOURCES {
            store
                .register_remote(&format!("source-{index}"), address.clone())
                .unwrap();
        }
        assert!(store
            .register_remote("one-too-many", address.clone())
            .is_err());
        assert!(store
            .register_remote(&"x".repeat(MAX_SOURCE_LABEL_CHARS + 1), address)
            .is_err());
        assert!(matches!(
            SourceRegistry::parse(&"x".repeat(MAX_REGISTRY_BYTES + 1)),
            Err(Error::TooLarge { .. })
        ));

        // Five full sources exceed the 2048-entry merged catalog budget.
        let sources = root.join("sources");
        fs::create_dir_all(&sources).unwrap();
        let mut store = DiscoveryStore::open(root.join("budget-state")).unwrap();
        for source_index in 0..5 {
            let path = sources.join(format!("source-{source_index}.ylang"));
            let entries: Vec<(String, String)> = (0..512)
                .map(|entry_index| {
                    (
                        format!("Entry {source_index}-{entry_index}"),
                        format!("p{source_index}-{entry_index}"),
                    )
                })
                .collect();
            make_source(&path, &format!("Source {source_index}"), &entries);
            store
                .register_local(&format!("source-{source_index}"), &path)
                .unwrap();
        }
        let view = store.rebuild().unwrap();
        assert_eq!(view.len(), MAX_TOTAL_ENTRIES);
        assert!(view
            .statuses()
            .iter()
            .any(|status| matches!(status.state, SourceState::Unavailable { .. })));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn status_codec_round_trips_and_sanitizes_reasons() {
        for state in [
            SourceState::Fresh,
            SourceState::Cached {
                reason: "café unavailable\nline".into(),
            },
            SourceState::Unavailable {
                reason: "bad snapshot".into(),
            },
            SourceState::InactiveRemote,
            SourceState::NotBuilt,
        ] {
            let encoded = encode_status(&state);
            let decoded = parse_status(&encoded).unwrap().unwrap();
            match (state, decoded) {
                (SourceState::Fresh, SourceState::Fresh) => {}
                (SourceState::Cached { .. }, SourceState::Cached { reason }) => {
                    assert!(reason.contains("café unavailable"));
                    assert!(!reason.contains('\n'));
                }
                (SourceState::Unavailable { .. }, SourceState::Unavailable { reason }) => {
                    assert_eq!(reason, "bad snapshot");
                }
                (SourceState::InactiveRemote, SourceState::InactiveRemote) => {}
                (SourceState::NotBuilt, SourceState::NotBuilt) => {}
                (_, other) => panic!("unexpected decoded state: {other:?}"),
            }
        }
        assert!(parse_status("bad").is_err());
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(unhex("000fff"), Some(vec![0, 15, 255]));
        assert_eq!(unhex("0"), None);
    }
}
