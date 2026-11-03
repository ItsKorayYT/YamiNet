#![forbid(unsafe_code)]
pub mod archive;
pub mod liveness;
pub mod merkle;
pub mod polynomial;
mod relation;
pub type Result<T, E = Error> = std::result::Result<T, E>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Authorization,
    Membership,
    Overflow,
    Capacity,
    Invalid,
    Malformed,
    Crypto,
    Stale,
    Conflict,
    Recovery,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
#[cfg(test)]
mod tests;
