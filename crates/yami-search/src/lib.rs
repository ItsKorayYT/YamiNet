//! Bounded local full-text search of inert YLang ASTs. No network on query.
mod engine;
mod files;
mod provider;
mod publication;
mod text;

pub use engine::{Index, Page, SearchHit, Stats};
pub use provider::Provider;
pub use publication::{
    Manifest, Publication, ShardRef, MANIFEST_KEY, MAX_PUBLICATION_BYTES, MAX_SHARDS,
};
pub use text::{normalize, tokens};
pub use yami_discover::WebsiteAddress;

use std::io;
pub type Result<T> = io::Result<T>;
pub const FORMAT_VERSION: u32 = 1;
pub const MAX_PAGES: usize = 256;
pub const MAX_PAGE_TEXT: usize = 64 * 1024;
pub const MAX_TOTAL_TEXT: usize = 4 * 1024 * 1024;
pub const MAX_INDEX_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_TERMS: usize = 65_536;
pub const MAX_POSTINGS: usize = 262_144;
pub const MAX_RESULTS: usize = 100;
pub const MAX_QUERY_CHARS: usize = 128;
pub const MAX_QUERY_TERMS: usize = 8;

pub(crate) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
