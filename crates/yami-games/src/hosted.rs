//! Strict hosted formats: ABI 1 embedded resources, ABI 2 declared PNG/PCM assets.
use crate::*;
use sha2::{Digest, Sha256};
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleResource {
    pub key: String,
    pub bytes: usize,
    pub sha256: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedManifest {
    pub version: u32,
    pub abi: u32,
    pub title: String,
    pub description: String,
    pub module: ModuleResource,
    #[serde(default)]
    pub assets: Vec<AssetResource>,
}
impl HostedManifest {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() || bytes.len() > ynp::MAX_GAME_MANIFEST {
            return Err("Hosted manifest exceeds byte bounds".into());
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid hosted manifest: {e}"))?;
        if !matches!((manifest.version, manifest.abi), (1, 1) | (2, 2)) {
            return Err("Unsupported hosted manifest or ABI version".into());
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
            return Err("Invalid hosted title or description".into());
        }
        ynp::validate_asset_key(&manifest.module.key).map_err(|e| e.to_string())?;
        if manifest.module.bytes == 0
            || manifest.module.bytes > MAX_MODULE_BYTES
            || manifest.module.sha256.len() != 64
            || !manifest
                .module
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Invalid module size or SHA-256 digest".into());
        }
        assets::validate_resources(manifest.version, &manifest.assets)?;
        Ok(manifest)
    }
    /// Integrity is relative to the manifest delivered by the authenticated publisher.
    pub fn validate_module(&self, bytes: &[u8]) -> Result<(), String> {
        let digest: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if bytes.len() != self.module.bytes
            || digest != self.module.sha256
            || !bytes.starts_with(b"\0asm\x01\0\0\0")
        {
            return Err("Hosted module length, integrity or Wasm header mismatch".into());
        }
        Ok(())
    }
    pub fn runtime_manifest(self) -> Manifest {
        Manifest {
            version: self.version,
            abi: self.abi,
            title: self.title,
            description: self.description,
            module: self.module.key,
            assets: self.assets,
            prepared: std::sync::Arc::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> serde_json::Value {
        let bytes = include_bytes!("../../../games/breakout/breakout.wasm");
        let hash: String = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        serde_json::json!({"version":1,"abi":1,"title":"Prism","description":"Test","module":{"key":"prism","bytes":bytes.len(),"sha256":hash}})
    }
    #[test]
    fn hosted_manifest_is_strict_bounded_and_checks_integrity_before_vm() {
        let value = document();
        let good = serde_json::to_vec(&value).unwrap();
        let manifest = HostedManifest::parse(&good).unwrap();
        let bytes = include_bytes!("../../../games/breakout/breakout.wasm");
        manifest.validate_module(bytes).unwrap();
        let mut corrupt = bytes.to_vec();
        corrupt[100] ^= 1;
        assert!(manifest.validate_module(&corrupt).is_err());
        assert!(manifest.validate_module(&bytes[..bytes.len() - 1]).is_err());
        for (path, bad) in [
            ("version", serde_json::json!(2)),
            ("abi", serde_json::json!(2)),
            ("title", serde_json::json!("")),
            ("description", serde_json::json!("a\nb")),
            ("resources", serde_json::json!([])),
        ] {
            let mut value = document();
            value[path] = bad;
            assert!(HostedManifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        for (field, bad) in [
            ("key", serde_json::json!("../private")),
            ("key", serde_json::json!("yami://other/a")),
            ("bytes", serde_json::json!(MAX_MODULE_BYTES + 1)),
            ("bytes", serde_json::json!(0)),
            ("sha256", serde_json::json!("g".repeat(64))),
            ("sha256", serde_json::json!("ab")),
            ("url", serde_json::json!("x")),
        ] {
            let mut value = document();
            value["module"][field] = bad;
            assert!(HostedManifest::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        }
        assert!(HostedManifest::parse(&vec![b' '; ynp::MAX_GAME_MANIFEST + 1]).is_err());
        assert!(HostedManifest::parse(b"{}").is_err());
        let duplicate =
            String::from_utf8(good)
                .unwrap()
                .replacen("\"abi\":1", "\"abi\":1,\"abi\":1", 1);
        assert!(HostedManifest::parse(duplicate.as_bytes()).is_err());
    }
}
