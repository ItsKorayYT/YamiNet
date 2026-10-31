//! A standalone Language Server Protocol implementation for YamiNet's native
//! YLang and YSL document languages.
//!
//! The server is deliberately self-contained: it has no dependencies other than
//! the two language crates in this repository, performs no network requests and
//! keeps stdout reserved for JSON-RPC messages.

pub mod completion;
pub mod definition;
pub mod diagnostics;
pub mod documents;
pub mod hover;
pub mod json;
pub mod server;
pub mod site;
pub mod spec;
pub mod text;
pub mod transport;

pub use server::{run_stdio, Server};
