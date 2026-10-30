//! Versioned YLang 0.2 directory documents.
use crate::{
    Error, Result, WebsiteAddress, DIRECTORY_FORMAT_VERSION, MAX_CATEGORY_CHARS,
    MAX_DESCRIPTION_CHARS, MAX_DIRECTORY_BYTES, MAX_ENTRIES_PER_DIRECTORY, MAX_NAME_CHARS,
    MAX_TITLE_CHARS,
};
use std::{
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
    entries: Vec<DirectoryEntry>,
}

impl Directory {
    /// Creates a version-1 directory from already validated entries.
    pub fn new(name: impl Into<String>, entries: Vec<DirectoryEntry>) -> Result<Self> {
        let name =
            field_text(&name.into(), "directory title", MAX_NAME_CHARS).map_err(Error::invalid)?;
        if entries.len() > MAX_ENTRIES_PER_DIRECTORY {
            return Err(Error::limit(format!(
                "directory has {} entries; the maximum is {MAX_ENTRIES_PER_DIRECTORY}",
                entries.len()
            )));
        }
        Ok(Self {
            name,
            version: DIRECTORY_FORMAT_VERSION,
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
            address,
            Some("x".repeat(MAX_CATEGORY_CHARS + 1))
        )
        .is_err());
    }
}
