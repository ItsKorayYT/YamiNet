//! Fixed-validator local Malachite adapter. NOT production-ready; no real money.
mod node;
mod signing;
mod types;
mod wire;
pub use node::*;
pub use types::*;
pub use wire::{decode_certificate, encode_certificate, verify_record};
#[cfg(test)]
mod tests;
