//! Non-authoritative test TLDs. Never consulted by YNS or browser resolution.
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestNames(BTreeMap<String, [u8; 32]>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamingError {
    Reserved,
    InvalidLabel,
    NotTestLabel,
    AlreadyRegistered,
}

pub fn validate_label(label: &str) -> Result<(), NamingError> {
    if label.trim_start_matches('.').eq_ignore_ascii_case("yami") {
        return Err(NamingError::Reserved);
    }
    if label.is_empty()
        || label.len() > 63
        || !label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || label.starts_with('-')
        || label.ends_with('-')
    {
        return Err(NamingError::InvalidLabel);
    }
    if !label.starts_with("test-") || label.len() == 5 {
        return Err(NamingError::NotTestLabel);
    }
    Ok(())
}

impl TestNames {
    pub fn register(&mut self, label: &str, owner: [u8; 32]) -> Result<(), NamingError> {
        validate_label(label)?;
        if self.0.contains_key(label) {
            return Err(NamingError::AlreadyRegistered);
        }
        self.0.insert(label.to_owned(), owner);
        Ok(())
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8; 32])> {
        self.0.iter().map(|(name, owner)| (name.as_str(), owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservation_and_canonical_labels() {
        for label in ["yami", ".yami", "YAMI", ".YaMi"] {
            assert_eq!(validate_label(label), Err(NamingError::Reserved));
        }
        for label in [
            "", "test-", "test-x.", "test-Ä", "test-A", "test-x-", "-test-x",
        ] {
            assert!(validate_label(label).is_err());
        }
        assert_eq!(validate_label("web"), Err(NamingError::NotTestLabel));
        let mut names = TestNames::default();
        names.register("test-local", [1; 32]).unwrap();
        assert_eq!(
            names.register("test-local", [2; 32]),
            Err(NamingError::AlreadyRegistered)
        );
    }
}
