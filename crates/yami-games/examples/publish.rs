//! Publisher-side preparation only; players never run this command.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("Usage: publish LOCAL-MANIFEST SITE-DIRECTORY GAME-KEY".into());
    }
    let key = args[2].to_str().ok_or("Key is not UTF-8")?;
    ynp::validate_asset_key(key)?;
    let (manifest, bytes) = yami_games::load_local(&PathBuf::from(&args[0]))?;
    yami_games::Runtime::with_assets(&bytes, Some(manifest.abi), manifest.prepared.clone())?
        .frame(0, 0)?;
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let mut assets = manifest.assets.clone();
    for asset in &mut assets {
        asset.key = format!("{key}-{}", asset.id);
        ynp::validate_asset_key(&asset.key)?;
    }
    let json = serde_json::to_vec_pretty(
        &serde_json::json!({"version":manifest.version,"abi":manifest.abi,"title":manifest.title,"description":manifest.description,"module":{"key":key,"bytes":bytes.len(),"sha256":hash},"assets":assets}),
    )?;
    yami_games::HostedManifest::parse(&json)?.validate_module(&bytes)?;
    let directory = fs::canonicalize(&args[1])?.join("games");
    if !directory.exists() {
        fs::create_dir(&directory)?;
    }
    if !fs::symlink_metadata(&directory)?.file_type().is_dir() {
        return Err("games must be a regular non-symlink directory".into());
    }
    // Never replace an existing publication implicitly. Manifest appears last.
    let parent = fs::canonicalize(&args[0])?
        .parent()
        .ok_or("Manifest has no directory")?
        .to_owned();
    for (source, target) in manifest.assets.iter().zip(&assets) {
        let path = parent
            .join(&source.key)
            .with_extension(source.kind.extension())
            .canonicalize()?;
        if !path.starts_with(&parent) {
            return Err("Asset escaped package".into());
        }
        if !path.is_file() {
            return Err("Asset must be a regular file".into());
        }
        let file = fs::File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err("Asset must be a regular file".into());
        }
        let mut data = Vec::new();
        file.take((source.kind.wire().limit() + 1) as u64)
            .read_to_end(&mut data)?;
        yami_games::Assets::default().insert(source, &data)?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join(format!("{}.{}", target.key, target.kind.extension())))?
            .write_all(&data)?;
    }
    let mut module = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("{key}.wasm")))?;
    module.write_all(&bytes)?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(format!("{key}.json")))?
        .write_all(&json)?;
    println!(
        "Published {key} ({} Wasm bytes). Add > game:{key} | {} to an @page 0.2 page.",
        bytes.len(),
        manifest.title
    );
    Ok(())
}
