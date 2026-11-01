//! Bounded WebAssembly games. No WASI or native plugins.
mod assets;
pub use assets::{
    AssetKind, AssetResource, Assets, Sound, MAX_ASSETS, MAX_ASSET_TOTAL, MAX_DECODED_TOTAL,
};
mod audio;
pub use audio::AudioCommand;
mod hosted;
pub use hosted::{HostedManifest, ModuleResource};
mod runtime;
mod session;
pub use runtime::{Command, Frame, Runtime};
use serde::Deserialize;
pub use session::{Session, Snapshot, Status};
use std::{
    fs::File,
    io::Read,
    path::{Component, Path},
};

pub const ABI_VERSION: i32 = 2;
pub const UP: u32 = 16;
pub const DOWN: u32 = 32;
pub const ACTION: u32 = 64;
pub const SECONDARY: u32 = 128;
pub const MOUSE_LEFT: u32 = 256;
pub const MOUSE_RIGHT: u32 = 512;
pub const INPUT_MASK: u32 = 1023;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Input {
    pub held: u32,
    pub pressed: u32,
    pub released: u32,
    /// (-1, -1) means the pointer is outside the game.
    pub x: i32,
    pub y: i32,
}
pub const WIDTH: i32 = 640;
pub const HEIGHT: i32 = 480;
pub const MAX_MODULE_BYTES: usize = 1024 * 1024;
pub const MAX_MEMORY_BYTES: usize = 4 * 1024 * 1024;
pub const FRAME_FUEL: u64 = 200_000;
pub const MAX_COMMANDS: usize = 512;
pub const LEFT: u32 = 1;
pub const RIGHT: u32 = 2;
pub const START: u32 = 4;
pub const RESET: u32 = 8;
pub const TICK_US: i32 = 16_667;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: u32,
    pub abi: u32,
    pub title: String,
    pub description: String,
    pub module: String,
    #[serde(default)]
    pub assets: Vec<AssetResource>,
    #[serde(skip)]
    pub prepared: std::sync::Arc<Assets>,
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|e| format!("Cannot open game file: {e}"))?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Game content must be a regular local file".into());
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err("Game file exceeds size limit".into());
    }
    Ok(bytes)
}

/// Executable content is only reached from an explicit local game action.
/// A module reference is a plain relative path confined to the manifest directory.
pub fn load_local(path: &Path) -> Result<(Manifest, Vec<u8>), String> {
    // Check before opening too, so obvious pipes/devices are never read.
    if !path.is_file() {
        return Err("Choose a regular local game manifest".into());
    }
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let mut manifest: Manifest = serde_json::from_slice(&read_bounded(&path, 4096)?)
        .map_err(|e| format!("Invalid game manifest: {e}"))?;
    if !matches!((manifest.version, manifest.abi), (1, 1) | (2, 2)) {
        return Err("Unsupported game manifest or ABI version".into());
    }
    if manifest.title.is_empty()
        || manifest.title.len() > 80
        || manifest.description.len() > 512
        || manifest
            .title
            .chars()
            .chain(manifest.description.chars())
            .any(char::is_control)
    {
        return Err("Invalid game title or description".into());
    }
    assets::validate_resources(manifest.version, &manifest.assets)?;
    let reference = Path::new(&manifest.module);
    if manifest.module.is_empty()
        || manifest.module.contains([':', '\\'])
        || reference
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Game module must be a relative local path without traversal".into());
    }
    let parent = path.parent().ok_or("Manifest has no directory")?;
    let module = parent
        .join(reference)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !module.starts_with(parent) || !module.is_file() {
        return Err("Game module must remain inside the manifest directory".into());
    }
    let mut prepared = Assets::default();
    for asset in &manifest.assets {
        let resource = parent
            .join(&asset.key)
            .with_extension(asset.kind.extension())
            .canonicalize()
            .map_err(|e| e.to_string())?;
        if !resource.starts_with(parent) || !resource.is_file() {
            return Err("Asset must remain inside the manifest directory".into());
        }
        prepared.insert(asset, &read_bounded(&resource, asset.kind.wire().limit())?)?;
    }
    manifest.prepared = std::sync::Arc::new(prepared);
    Ok((manifest, read_bounded(&module, MAX_MODULE_BYTES)?))
}
