use crate::{files, invalid, Index, Page, Publication, Result, FORMAT_VERSION, MAX_INDEX_BYTES};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

/// Explicit operator-supplied imports only; this type never crawls or fetches.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    version: u32,
    pages: Vec<Page>,
}
impl Provider {
    pub fn create(directory: impl AsRef<Path>) -> Result<()> {
        let directory = directory.as_ref();
        files::check_path(directory)?;
        std::fs::create_dir(directory)?;
        Self {
            version: FORMAT_VERSION,
            pages: Vec::new(),
        }
        .save(directory)
    }
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let text = files::read(&directory.as_ref().join("provider.json"), MAX_INDEX_BYTES)?;
        let state: Self = serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
        if state.version != FORMAT_VERSION {
            return Err(invalid("unsupported provider state version"));
        }
        Index::from_pages(state.pages.clone())?;
        if state.pages.iter().any(|p| p.address.is_none()) {
            return Err(invalid("provider pages require canonical addresses"));
        }
        Ok(state)
    }
    /// Identical canonical duplicates collapse; conflicting claims are rejected.
    pub fn add(&mut self, index: &Index) -> Result<()> {
        let mut pages: BTreeMap<_, _> = self.pages.iter().map(|p| (p.id(), p.clone())).collect();
        for page in index.pages() {
            if page.address.is_none() {
                return Err(invalid(
                    "provider imports require supplied website identities",
                ));
            }
            if let Some(previous) = pages.insert(page.id(), page.clone()) {
                if previous != *page {
                    return Err(invalid("conflicting duplicate page claim"));
                }
            }
        }
        let pages: Vec<_> = pages.into_values().collect();
        Index::from_pages(pages.clone())?;
        self.pages = pages;
        Ok(())
    }
    pub fn save(&self, directory: impl AsRef<Path>) -> Result<()> {
        let text = serde_json::to_string(self).map_err(|e| invalid(e.to_string()))?;
        if text.len() > MAX_INDEX_BYTES {
            return Err(invalid("provider state exceeds budget"));
        }
        files::write(&directory.as_ref().join("provider.json"), &text)
    }
    pub fn build(&self) -> Result<Publication> {
        Publication::build(&Index::from_pages(self.pages.clone())?)
    }
}
