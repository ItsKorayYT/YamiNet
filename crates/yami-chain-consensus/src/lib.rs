//! Height-specific local Malachite adapter. NOT production-ready; no real money.
mod candidates;
mod checkpoint;
mod compact;
pub use checkpoint::*;
pub use compact::*;
pub mod network;
mod node;
mod signing;
mod types;
mod wire;
pub use node::*;
pub use types::*;
pub use wire::{decode_certificate, encode_certificate, verify_record};
#[cfg(test)]
mod tests;

#[cfg(test)]
mod staking_sim;

#[cfg(any(test, feature = "fuzzing"))]
mod fuzzing;
#[cfg(feature = "fuzzing")]
pub use fuzzing::{fuzz_decode, fuzz_seeds};
