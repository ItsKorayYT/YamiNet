//! Versioned YLang 0.2 directory documents.
use crate::{
    store::{self, Mode},
    Error, Result, WebsiteAddress, DIRECTORY_FORMAT_VERSION, MAX_CATEGORY_CHARS,
    MAX_DESCRIPTION_CHARS, MAX_DIRECTORY_BYTES, MAX_ENTRIES_PER_DIRECTORY, MAX_NAME_CHARS,
    MAX_TITLE_CHARS,
};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::Read,
    path::Path,
};
use ylang::{ContainerKind, Node, NodeKind, TextRole, Version};

/// One validated website listing.
///
/// The fields are intentionally immutable through this type so that a
/// [`Directory`] cannot silently drift from the values that were validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryEntry {
    title: String,
    description: String,
    address: WebsiteAddress,
    category: Option<String>,
}

impl DirectoryEntry {
    /// Creates a validated listing.
    pub fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        address: WebsiteAddress,
        category: Option<String>,
    ) -> Result<Self> {
        let title = field_text(&title.into(), "title", MAX_TITLE_CHARS).map_err(Error::invalid)?;
        let description = field_text(&description.into(), "description", MAX_DESCRIPTION_CHARS)
            .map_err(Error::invalid)?;
        let category = match category {
            Some(category) => Some(
                field_text(&category, "category", MAX_CATEGORY_CHARS).map_err(Error::invalid)?,
            ),
            None => None,
        };
        Ok(Self {
            title,
            description,
            address,
            category,
        })
    }

    /// The human-readable website title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The short, plain-text description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The canonical `yami://` destination. This is not a trust proof.
    pub fn address(&self) -> &WebsiteAddress {
        &self.address
    }

    /// The optional free-form category.
    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }
}

/// A parsed and validated directory document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directory {
    name: String,
    version: u32,
    language: Option<String>,
    entries: Vec<DirectoryEntry>,
}

impl Directory {
    /// Creates a version-1 directory from already validated entries.
    ///
    /// Duplicate canonical addresses are rejected. Parsing remains tolerant of
    /// duplicates so [`crate::Catalog`] can still count and deduplicate them
    /// when importing an independently authored source.
    pub fn new(name: impl Into<String>, entries: Vec<DirectoryEntry>) -> Result<Self> {
        let name =
            field_text(&name.into(), "directory title", MAX_NAME_CHARS).map_err(Error::invalid)?;
        if entries.len() > MAX_ENTRIES_PER_DIRECTORY {
            return Err(Error::limit(format!(
                "directory has {} entries; the maximum is {MAX_ENTRIES_PER_DIRECTORY}",
                entries.len()
            )));
        }
        let mut seen = HashSet::new();
        for entry in &entries {
            if !seen.insert(entry.address().clone()) {
                return Err(Error::conflict(format!(
                    "duplicate website address {}",
                    entry.address()
                )));
            }
        }
        Ok(Self {
            name,
            version: DIRECTORY_FORMAT_VERSION,
            language: None,
            entries,
        })
    }

    /// Parses and validates a directory document.
    ///
    /// The source must be a YLang 0.2 document whose metadata title names the
    /// directory and whose only top-level block is `:: directory 1`.
    pub fn parse(source: &str) -> Result<Self> {
        if source.len() > MAX_DIRECTORY_BYTES {
            return Err(Error::TooLarge {
                limit: MAX_DIRECTORY_BYTES,
            });
        }
        let document = ylang::parse(source)?;
        if document.version != Version::V02 {
            return Err(Error::invalid(
                "directory documents require the @page 0.2 YLang format",
            ));
        }
        let name = field_text(&document.metadata.title, "directory title", MAX_NAME_CHARS)
            .map_err(Error::invalid)?;
        let language = document.metadata.language.clone();
        let node = match document.nodes.as_slice() {
            [] => return Err(Error::invalid("expected one :: directory 1 section")),
            [node] => node,
            [_, extra, ..] => {
                return Err(Error::at(
                    &extra.span,
                    "only one :: directory 1 section is allowed at top level",
                ))
            }
        };
        let NodeKind::Section {
            name: section,
            children,
        } = &node.kind
        else {
            return Err(Error::at(&node.span, "expected a :: directory 1 section"));
        };
        let version = parse_version(section).map_err(|message| Error::at(&node.span, message))?;
        if children.len() > MAX_ENTRIES_PER_DIRECTORY {
            return Err(Error::at(
                &children[MAX_ENTRIES_PER_DIRECTORY].span,
                format!("directory exceeds {MAX_ENTRIES_PER_DIRECTORY} entries"),
            ));
        }
        let mut entries = Vec::with_capacity(children.len());
        for (index, child) in children.iter().enumerate() {
            entries.push(parse_entry(child, index + 1)?);
        }
        Ok(Self {
            name,
            version,
            language,
            entries,
        })
    }

    /// Loads a bounded, regular, non-symlink UTF-8 directory file.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(Error::invalid(
                "directory must be a regular, non-symlink UTF-8 file",
            ));
        }
        if metadata.len() > MAX_DIRECTORY_BYTES as u64 {
            return Err(Error::TooLarge {
                limit: MAX_DIRECTORY_BYTES,
            });
        }
        let mut source = String::new();
        File::open(path)?
            .take((MAX_DIRECTORY_BYTES + 1) as u64)
            .read_to_string(&mut source)?;
        Self::parse(&source)
    }

    /// The directory's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The directory format version (currently always 1).
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The listings in document order. Duplicates are not removed here; the
    /// merged [`crate::Catalog`] deduplicates canonical addresses.
    pub fn entries(&self) -> &[DirectoryEntry] {
        &self.entries
    }

    /// The optional YLang `language` metadata, preserved across a rewrite.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Looks up one entry by its canonical address.
    pub fn entry(&self, address: &WebsiteAddress) -> Option<&DirectoryEntry> {
        self.entries.iter().find(|entry| entry.address() == address)
    }

    /// Appends one entry in deterministic document order.
    ///
    /// Duplicate canonical addresses and the per-directory entry cap are
    /// rejected without changing the directory.
    pub fn add_entry(&mut self, entry: DirectoryEntry) -> Result<()> {
        if self.entries.len() >= MAX_ENTRIES_PER_DIRECTORY {
            return Err(Error::limit(format!(
                "directory already has {MAX_ENTRIES_PER_DIRECTORY} entries"
            )));
        }
        if self
            .entries
            .iter()
            .any(|existing| existing.address() == entry.address())
        {
            return Err(Error::conflict(format!(
                "duplicate website address {}",
                entry.address()
            )));
        }
        self.entries.push(entry);
        Ok(())
    }

    /// Removes and returns the entry with this canonical address.
    pub fn remove_entry(&mut self, address: &WebsiteAddress) -> Result<DirectoryEntry> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.address() == address)
            .ok_or_else(|| Error::invalid(format!("no entry with address {address}")))?;
        Ok(self.entries.remove(index))
    }

    /// Replaces one entry in place, preserving its position.
    ///
    /// Changing the replacement's address is allowed only when the new address
    /// is not already used by another entry.
    pub fn replace_entry(
        &mut self,
        address: &WebsiteAddress,
        replacement: DirectoryEntry,
    ) -> Result<()> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.address() == address)
            .ok_or_else(|| Error::invalid(format!("no entry with address {address}")))?;
        if replacement.address() != address
            && self
                .entries
                .iter()
                .enumerate()
                .any(|(other, entry)| other != index && entry.address() == replacement.address())
        {
            return Err(Error::conflict(format!(
                "duplicate website address {}",
                replacement.address()
            )));
        }
        self.entries[index] = replacement;
        Ok(())
    }

    /// Renders a canonical YLang 0.2 directory document.
    ///
    /// Descriptions are emitted as YLang literal lines (a leading backslash),
    /// so reserved prefixes, brackets and backslashes in user text survive a
    /// parse unchanged. The rendered document is rejected if it would exceed
    /// [`MAX_DIRECTORY_BYTES`].
    pub fn render(&self) -> Result<String> {
        let mut source = String::new();
        source.push_str("@page 0.2\n  title ");
        source.push_str(&self.name);
        source.push('\n');
        if let Some(language) = &self.language {
            source.push_str("  language ");
            source.push_str(language);
            source.push('\n');
        }
        source.push_str("\n:: directory 1\n");
        for entry in &self.entries {
            source.push_str("\n  :: card ");
            source.push_str(entry.title());
            source.push_str("\n    > ");
            source.push_str(&entry.address().to_string());
            source.push_str(" | Open\n    \\");
            source.push_str(entry.description());
            source.push('\n');
            if let Some(category) = entry.category() {
                source.push_str("    :: category ");
                source.push_str(category);
                source.push('\n');
            }
        }
        if source.len() > MAX_DIRECTORY_BYTES {
            return Err(Error::TooLarge {
                limit: MAX_DIRECTORY_BYTES,
            });
        }
        Ok(source)
    }

    /// Atomically replaces an existing regular directory file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let source = self.checked_source()?;
        store::write_atomic(path.as_ref(), &source, Mode::Replace)
    }

    /// Atomically creates a new directory file without overwriting any path.
    pub fn save_new(&self, path: impl AsRef<Path>) -> Result<()> {
        let source = self.checked_source()?;
        store::write_atomic(path.as_ref(), &source, Mode::CreateNew)
    }

    fn checked_source(&self) -> Result<String> {
        let source = self.render()?;
        let parsed = Self::parse(&source)?;
        if &parsed != self {
            return Err(Error::invalid(
                "generated directory failed its round-trip check; refusing to write",
            ));
        }
        Ok(source)
    }
}

fn parse_version(section: &str) -> std::result::Result<u32, String> {
    let text = section
        .strip_prefix("directory ")
        .ok_or_else(|| "expected a :: directory 1 wrapper".to_string())?;
    let version: u32 = text
        .parse()
        .map_err(|_| "directory format version must be an integer".to_string())?;
    if text != version.to_string() {
        return Err("directory format version must not contain leading zeros".into());
    }
    if version != DIRECTORY_FORMAT_VERSION {
        return Err(format!(
            "unsupported directory format version {version}; this build reads version {DIRECTORY_FORMAT_VERSION}"
        ));
    }
    Ok(version)
}

fn parse_entry(node: &Node, index: usize) -> Result<DirectoryEntry> {
    let context = |message: &str| format!("entry {index}: {message}");
    let NodeKind::Container {
        kind: ContainerKind::Card,
        name: title,
        children,
    } = &node.kind
    else {
        return Err(Error::at(
            &node.span,
            context("expected a `:: card Title` entry"),
        ));
    };
    let title = field_text(title, "title", MAX_TITLE_CHARS)
        .map_err(|message| Error::at(&node.span, context(&message)))?;
    let mut address = None;
    let mut description = None;
    let mut category = None;
    for child in children {
        match &child.kind {
            NodeKind::Link { target, .. } => {
                if address.is_some() {
                    return Err(Error::at(
                        &child.span,
                        context("entry has more than one address link"),
                    ));
                }
                address = Some(WebsiteAddress::parse(target).map_err(|error| {
                    Error::at(&child.span, context(&format!("{error}")))
                })?);
            }
            NodeKind::Text {
                role: TextRole::Paragraph,
                content,
            } => {
                if description.is_some() {
                    return Err(Error::at(
                        &child.span,
                        context("entry has more than one description paragraph"),
                    ));
                }
                let text = ylang::plain_text(content);
                let text = field_text(&text, "description", MAX_DESCRIPTION_CHARS)
                    .map_err(|message| Error::at(&child.span, context(&message)))?;
                description = Some(text);
            }
            NodeKind::Text {
                role: TextRole::Heading(_),
                ..
            } => {
                return Err(Error::at(
                    &child.span,
                    context("headings are not allowed inside an entry"),
                ))
            }
            NodeKind::Section { name, children } => {
                let value = name.strip_prefix("category ").ok_or_else(|| {
                    Error::at(
                        &child.span,
                        context("only an optional `:: category NAME` section is allowed"),
                    )
                })?;
                if !children.is_empty() {
                    return Err(Error::at(
                        &child.span,
                        context("a category section cannot contain child blocks"),
                    ));
                }
                if category.is_some() {
                    return Err(Error::at(
                        &child.span,
                        context("entry has more than one category"),
                    ));
                }
                let value = field_text(value, "category", MAX_CATEGORY_CHARS)
                    .map_err(|message| Error::at(&child.span, context(&message)))?;
                category = Some(value);
            }
            _ => {
                return Err(Error::at(
                    &child.span,
                    context(
                        "only one yami:// link, one description paragraph and an optional category are allowed",
                    ),
                ))
            }
        }
    }
    let address = address.ok_or_else(|| {
        Error::at(
            &node.span,
            context("entry is missing a yami:// address link"),
        )
    })?;
    let description = description.ok_or_else(|| {
        Error::at(
            &node.span,
            context("entry is missing a description paragraph"),
        )
    })?;
    Ok(DirectoryEntry {
        title,
        description,
        address,
        category,
    })
}

fn field_text(value: &str, label: &str, max: usize) -> std::result::Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} cannot be empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} contains control characters"));
    }
    if value.chars().count() > max {
        return Err(format!("{label} exceeds {max} characters"));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "yn1-sp2hcdqey6gfx62bqb3vzymxh4hjsjv2ocfqk4hecwplc77pkh2yatmgrq";

    #[test]
    fn field_and_version_validation() {
        assert_eq!(parse_version("directory 1"), Ok(1));
        assert!(parse_version("directory 2").is_err());
        assert!(parse_version("directory 01").is_err());
        assert!(parse_version("directory").is_err());
        assert!(parse_version("directory one").is_err());
        assert!(field_text("  ", "title", 10).is_err());
        assert!(field_text("abcdef", "title", 5).is_err());
        assert_eq!(field_text("  abc  ", "title", 5), Ok("abc".into()));
    }

    #[test]
    fn directory_entry_constructor_is_bounded() {
        let address = WebsiteAddress::parse(&format!("yami://{ID}/home")).unwrap();
        assert!(DirectoryEntry::new("", "description", address.clone(), None).is_err());
        assert!(DirectoryEntry::new("title", "", address.clone(), None).is_err());
        assert!(
            DirectoryEntry::new("title", "description", address.clone(), Some(" ".into())).is_err()
        );
        assert!(DirectoryEntry::new(
            "title",
            "description",
            address.clone(),
            Some("x".repeat(MAX_CATEGORY_CHARS + 1))
        )
        .is_err());
        assert!(DirectoryEntry::new("title\nline", "description", address.clone(), None).is_err());
        assert!(DirectoryEntry::new("title", "bad\tdescription", address, None).is_err());
    }

    fn address(key: &str) -> WebsiteAddress {
        WebsiteAddress::parse(&format!("yami://{ID}/{key}")).unwrap()
    }

    fn entry(title: &str, key: &str) -> DirectoryEntry {
        DirectoryEntry::new(title, "Description", address(key), None).unwrap()
    }

    #[test]
    fn rendering_round_trips_reserved_text_unicode_and_language() {
        let tricky = DirectoryEntry::new(
            ":: card > [Title] | \\",
            "  = heading [strong: literal] \\ --- @page 0.2 日本語  ",
            address("home"),
            Some(":: category [x] \\".into()),
        )
        .unwrap();
        let mut directory = Directory::new("Café & YamiNet [directory]", vec![tricky]).unwrap();
        directory.add_entry(entry("Second", "about")).unwrap();
        let source = directory.render().unwrap();
        assert!(source.contains("    \\"));
        assert_eq!(Directory::parse(&source).unwrap(), directory);

        let parsed =
            Directory::parse("@page 0.2\n  title Language\n  language en\n:: directory 1\n")
                .unwrap();
        assert_eq!(parsed.language(), Some("en"));
        let rendered = parsed.render().unwrap();
        assert!(rendered.contains("  language en\n"));
        assert_eq!(Directory::parse(&rendered).unwrap(), parsed);
    }

    #[test]
    fn mutations_preserve_order_and_reject_duplicates() {
        let first = entry("First", "home");
        let second = entry("Second", "about");
        let third = entry("Third", "notes");
        let mut directory = Directory::new("Ordered", vec![first.clone(), second.clone()]).unwrap();

        assert!(matches!(
            directory.add_entry(first.clone()),
            Err(Error::Conflict { .. })
        ));
        directory.add_entry(third.clone()).unwrap();
        let titles = |directory: &Directory| {
            directory
                .entries()
                .iter()
                .map(|entry| entry.title().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(titles(&directory), ["First", "Second", "Third"]);

        assert_eq!(
            directory.remove_entry(second.address()).unwrap().title(),
            "Second"
        );
        assert_eq!(titles(&directory), ["First", "Third"]);
        assert!(directory.remove_entry(second.address()).is_err());

        let replacement = DirectoryEntry::new(
            "First edited",
            "Updated description",
            address("home"),
            Some("Updated".into()),
        )
        .unwrap();
        directory
            .replace_entry(first.address(), replacement.clone())
            .unwrap();
        assert_eq!(titles(&directory), ["First edited", "Third"]);
        assert_eq!(
            directory.entry(first.address()).unwrap().category(),
            Some("Updated")
        );

        let duplicate = entry("Duplicate", "notes");
        assert!(matches!(
            directory.replace_entry(first.address(), duplicate),
            Err(Error::Conflict { .. })
        ));
        assert_eq!(directory.entries()[0].title(), "First edited");

        assert!(matches!(
            Directory::new("Bad", vec![first.clone(), first]),
            Err(Error::Conflict { .. })
        ));
    }

    #[test]
    fn add_entry_enforces_the_directory_entry_limit() {
        let mut directory = Directory::new("Full", vec![]).unwrap();
        for index in 0..MAX_ENTRIES_PER_DIRECTORY {
            directory
                .add_entry(entry(&format!("Entry {index}"), &format!("p{index}")))
                .unwrap();
        }
        assert!(matches!(
            directory.add_entry(entry("One too many", "extra")),
            Err(Error::Limit { .. })
        ));
    }

    #[test]
    fn render_rejects_documents_over_the_source_budget() {
        let title = "T".repeat(MAX_TITLE_CHARS);
        let description = "D".repeat(MAX_DESCRIPTION_CHARS);
        let category = "C".repeat(MAX_CATEGORY_CHARS);
        let entries = (0..450)
            .map(|index| {
                DirectoryEntry::new(
                    &title,
                    &description,
                    address(&format!("p{index}")),
                    Some(category.clone()),
                )
                .unwrap()
            })
            .collect();
        let directory = Directory::new("Large", entries).unwrap();
        assert!(matches!(directory.render(), Err(Error::TooLarge { .. })));
        let path = std::env::temp_dir().join(format!(
            "yami-discover-oversized-save-{}.ylang",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(directory.save_new(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn save_and_save_new_round_trip_without_leaving_temporary_files() {
        let directory =
            std::env::temp_dir().join(format!("yami-discover-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("community.ylang");

        let saved = Directory::new("Saved", vec![entry("First", "home")]).unwrap();
        saved.save_new(&path).unwrap();
        assert_eq!(Directory::load(&path).unwrap(), saved);
        assert!(saved.save_new(&path).is_err());
        assert_eq!(Directory::load(&path).unwrap(), saved);

        let changed = Directory::new("Changed", vec![]).unwrap();
        changed.save(&path).unwrap();
        assert_eq!(Directory::load(&path).unwrap(), changed);
        assert!(!std::fs::read_dir(&directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".yami-discover-")
        }));

        let _ = std::fs::remove_dir_all(&directory);
    }
}
