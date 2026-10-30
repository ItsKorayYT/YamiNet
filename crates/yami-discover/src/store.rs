//! Atomic file writes for authored directory documents.
//!
//! Content is written to a unique temporary file in the destination directory,
//! flushed, and only then exposed at the final path. A failed operation removes
//! its temporary file and leaves any existing target untouched.
use crate::{Error, Result};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Fail if the destination already exists.
    CreateNew,
    /// Atomically replace an existing regular file.
    Replace,
}

pub(crate) fn write_atomic(path: &Path, contents: &str, mode: Mode) -> Result<()> {
    let name = path
        .file_name()
        .ok_or_else(|| Error::invalid("path must name a file"))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    check_target(path, mode)?;

    let (temporary, mut file) = create_temporary(parent, name)?;
    let write_result = (|| -> io::Result<()> {
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary);
        return Err(error.into());
    }

    let commit = match mode {
        Mode::CreateNew => match fs::hard_link(&temporary, path) {
            Ok(()) => {
                // The destination is now complete and durable. Cleanup of the
                // temporary hard link is best-effort.
                let _ = fs::remove_file(&temporary);
                Ok(())
            }
            Err(error) => Err(error),
        },
        Mode::Replace => fs::rename(&temporary, path),
    };
    match commit {
        Ok(()) => {
            sync_parent(parent);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(error.into())
        }
    }
}

fn check_target(path: &Path, mode: Mode) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => match mode {
            Mode::CreateNew => Err(Error::invalid(format!(
                "refusing to overwrite existing path {}",
                path.display()
            ))),
            Mode::Replace if !metadata.file_type().is_file() => {
                Err(Error::invalid("target must be a regular, non-symlink file"))
            }
            Mode::Replace => Ok(()),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => match mode {
            Mode::CreateNew => Ok(()),
            // Replacing a missing file is treated as an error rather than
            // silently creating it at a mistyped path.
            Mode::Replace => Err(Error::from(error)),
        },
        Err(error) => Err(Error::from(error)),
    }
}

fn create_temporary(parent: &Path, name: &OsStr) -> Result<(PathBuf, File)> {
    let stem: String = name
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .take(48)
        .collect();
    let stem = if stem.is_empty() {
        "directory".to_string()
    } else {
        stem
    };
    for attempt in 0..64 {
        let temporary = parent.join(format!(
            ".{stem}.yami-discover-{}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::invalid(
        "could not allocate a temporary file for an atomic write",
    ))
}

#[cfg(unix)]
fn sync_parent(parent: &Path) {
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("yami-discover-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    fn leftover_temporaries(directory: &Path) -> usize {
        fs::read_dir(directory)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".yami-discover-")
            })
            .count()
    }

    #[test]
    fn creates_and_replaces_without_clobbering_unrelated_files() {
        let directory = temp_directory("atomic");
        let target = directory.join("community.ylang");
        let unrelated = directory.join("keep.txt");
        fs::write(&unrelated, "keep me").unwrap();

        write_atomic(&target, "first", Mode::CreateNew).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");
        assert!(write_atomic(&target, "second", Mode::CreateNew).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "first");

        write_atomic(&target, "second", Mode::Replace).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "second");
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "keep me");
        assert_eq!(leftover_temporaries(&directory), 0);

        assert!(write_atomic(&directory, "x", Mode::Replace).is_err());
        assert!(write_atomic(&directory.join("missing.ylang"), "x", Mode::Replace).is_err());
        assert!(!directory.join("missing.ylang").exists());
        assert_eq!(leftover_temporaries(&directory), 0);

        let _ = fs::remove_dir_all(directory);
    }
}
