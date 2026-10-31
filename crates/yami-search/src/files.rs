use crate::{invalid, Result};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

/// Paths are locally trusted; reject symlinks including parent components.
/// As with node serving, concurrent hostile filesystem mutation is unsupported.
pub(crate) fn check_path(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().filter(|p| !p.as_os_str().is_empty()) {
        if ancestor
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(invalid("parent traversal is forbidden"));
        }
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(invalid("symlinks are forbidden"))
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

pub(crate) fn read(path: &Path, limit: usize) -> Result<String> {
    check_path(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() || meta.len() > limit as u64 {
        return Err(invalid("expected a bounded regular non-symlink file"));
    }
    let mut text = String::new();
    File::open(path)?
        .take(limit as u64 + 1)
        .read_to_string(&mut text)?;
    if text.len() > limit {
        return Err(invalid("file exceeds byte budget"));
    }
    Ok(text)
}

pub(crate) fn write(path: &Path, text: &str) -> Result<()> {
    check_path(path)?;
    yami_discover::write_atomic_file(path, text).map_err(|e| invalid(e.to_string()))
}

pub(crate) fn directory(path: &Path) -> Result<()> {
    check_path(path)?;
    fs::create_dir_all(path)?;
    if !fs::symlink_metadata(path)?.file_type().is_dir() {
        return Err(invalid("expected directory"));
    }
    Ok(())
}
