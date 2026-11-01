//! Publisher-declared, eagerly verified resources. No guest-driven I/O or eviction races.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Cursor, sync::Arc};
pub const MAX_ASSETS: usize = 16;
pub const MAX_ASSET_TOTAL: usize = 4 * 1024 * 1024;
pub const MAX_DECODED_TOTAL: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Png,
    Wav,
}
impl AssetKind {
    pub fn wire(self) -> ynp::GameResource {
        match self {
            Self::Png => ynp::GameResource::Png,
            Self::Wav => ynp::GameResource::Wav,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Wav => "wav",
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetResource {
    pub id: u32,
    pub key: String,
    pub kind: AssetKind,
    pub bytes: usize,
    pub sha256: String,
}
pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn validate_resources(version: u32, resources: &[AssetResource]) -> Result<(), String> {
    if resources.len() > MAX_ASSETS || (version == 1 && !resources.is_empty()) {
        return Err("Unsupported asset list".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut keys = std::collections::BTreeSet::new();
    let mut total = 0;
    for r in resources {
        ynp::validate_asset_key(&r.key).map_err(|e| e.to_string())?;
        if r.id >= MAX_ASSETS as u32
            || !ids.insert(r.id)
            || !keys.insert((&r.key, r.kind.extension()))
            || r.bytes == 0
            || r.bytes > r.kind.wire().limit()
            || !valid_digest(&r.sha256)
        {
            return Err("Invalid asset ID, key, size or digest".into());
        }
        total += r.bytes;
    }
    if total > MAX_ASSET_TOTAL {
        return Err("Game asset byte budget exceeded".into());
    }
    Ok(())
}
#[derive(Debug, Clone)]
pub struct Sound {
    pub channels: u16,
    pub rate: u32,
    pub samples: Arc<[f32]>,
}
#[derive(Debug, Default)]
pub struct Assets {
    pub(crate) textures: BTreeMap<u32, tiny_skia::Pixmap>,
    pub(crate) sounds: BTreeMap<u32, Sound>,
    decoded: usize,
}
impl Assets {
    pub fn texture(&self, id: u32) -> Option<&tiny_skia::Pixmap> {
        self.textures.get(&id)
    }
    pub fn sound(&self, id: u32) -> Option<&Sound> {
        self.sounds.get(&id)
    }
    /// Call on a worker only. Integrity is checked before any decoder sees the resource.
    pub fn insert(&mut self, resource: &AssetResource, bytes: &[u8]) -> Result<(), String> {
        validate_resources(2, std::slice::from_ref(resource))?;
        if self.textures.len() + self.sounds.len() >= MAX_ASSETS
            || self.textures.contains_key(&resource.id)
            || self.sounds.contains_key(&resource.id)
        {
            return Err("Duplicate or excessive assets".into());
        }
        if bytes.len() != resource.bytes || digest(bytes) != resource.sha256 {
            return Err("Asset length or SHA-256 integrity mismatch".into());
        }
        let budget = MAX_DECODED_TOTAL - self.decoded;
        match resource.kind {
            AssetKind::Png => {
                let image =
                    yami_render::assets::decode_png(bytes, budget).map_err(|e| e.to_string())?;
                let size = tiny_skia::IntSize::from_wh(image.width, image.height)
                    .ok_or("Invalid texture size")?;
                let pixmap = tiny_skia::Pixmap::from_vec(image.pixels.to_vec(), size)
                    .ok_or("Invalid texture")?;
                self.decoded += pixmap.data().len();
                self.textures.insert(resource.id, pixmap);
            }
            AssetKind::Wav => {
                // A deliberately small format: PCM signed 16-bit mono/stereo, 8–48 kHz, <=10 s.
                if !bytes.starts_with(b"RIFF")
                    || bytes.get(8..12) != Some(b"WAVE")
                    || bytes.len() < 44
                    || u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize
                        != bytes.len() - 8
                {
                    return Err("Invalid WAV container".into());
                }
                let mut reader =
                    hound::WavReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
                let spec = reader.spec();
                let count = reader.len() as usize;
                if !(1..=2).contains(&spec.channels)
                    || !(8000..=48000).contains(&spec.sample_rate)
                    || spec.bits_per_sample != 16
                    || spec.sample_format != hound::SampleFormat::Int
                    || count == 0
                    || !count.is_multiple_of(spec.channels as usize)
                    || count > spec.sample_rate as usize * spec.channels as usize * 10
                    || count * 4 > budget
                {
                    return Err("Audio format, duration or decoded budget exceeded".into());
                }
                let samples: Vec<f32> = reader
                    .samples::<i16>()
                    .map(|s| s.map(|s| s as f32 / 32768.))
                    .collect::<Result<_, _>>()
                    .map_err(|e| e.to_string())?;
                if samples.len() != count {
                    return Err("Truncated audio samples".into());
                }
                self.decoded += samples.len() * 4;
                self.sounds.insert(
                    resource.id,
                    Sound {
                        channels: spec.channels,
                        rate: spec.sample_rate,
                        samples: samples.into(),
                    },
                );
            }
        }
        Ok(())
    }
}
