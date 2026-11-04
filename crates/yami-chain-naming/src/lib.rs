//! Non-authoritative test TLDs. Never consulted by YNS or browser resolution.
use im::OrdMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestNames {
    entries: OrdMap<String, [u8; 32]>,
    encoded_bytes: usize,
}

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
        if self.entries.contains_key(label) {
            return Err(NamingError::AlreadyRegistered);
        }
        self.insert_validated(label, owner);
        Ok(())
    }

    fn insert_validated(&mut self, label: &str, owner: [u8; 32]) {
        self.entries.insert(label.to_owned(), owner);
        self.encoded_bytes += 4 + label.len() + 32;
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Exact canonical entry-body size, rebuilt by validated registration.
    pub fn encoded_bytes(&self) -> usize {
        self.encoded_bytes
    }
    pub fn get(&self, label: &str) -> Option<&[u8; 32]> {
        self.entries.get(label)
    }
    pub fn entries(&self) -> impl Iterator<Item = (&str, &[u8; 32])> {
        self.entries
            .iter()
            .map(|(name, owner)| (name.as_str(), owner))
    }
}

/// Fresh canonical reconstruction only: ordering proves that no existing key
/// can be overwritten. Contents remain owned after the borrowed input is gone.
#[derive(Default)]
pub struct SortedNamesBuilder<'a> {
    names: TestNames,
    previous: Option<&'a str>,
}
impl<'a> SortedNamesBuilder<'a> {
    pub fn push(&mut self, label: &'a str, owner: [u8; 32]) -> Result<(), NamingError> {
        if self.previous.is_some_and(|last| last >= label) {
            return Err(NamingError::InvalidLabel);
        }
        validate_label(label)?;
        self.names.insert_validated(label, owner);
        self.previous = Some(label);
        Ok(())
    }
    pub fn finish(self) -> TestNames {
        self.names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sorted_reconstruction_rejects_overwrites_disorder_and_invalid_labels() {
        let mut builder = SortedNamesBuilder::default();
        assert_eq!(builder.push("yami", [1; 32]), Err(NamingError::Reserved));
        builder.push("test-b", [1; 32]).unwrap();
        assert_eq!(
            builder.push("test-a", [2; 32]),
            Err(NamingError::InvalidLabel)
        );
        assert_eq!(
            builder.push("test-b", [2; 32]),
            Err(NamingError::InvalidLabel)
        );
        builder.push("test-c", [3; 32]).unwrap();
        let names = builder.finish();
        let mut ordinary = TestNames::default();
        ordinary.register("test-c", [3; 32]).unwrap();
        ordinary.register("test-b", [1; 32]).unwrap();
        assert_eq!(names, ordinary);
        assert_eq!(names.len(), 2);
        assert_eq!(names.encoded_bytes(), 84);
        assert_eq!(names.get("test-b"), Some(&[1; 32]));
        assert_eq!(names.get("test-a"), None);
    }
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
