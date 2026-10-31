//! Inert YLang envelope for canonical, chunked UTF-8 JSON page data.
//! Hashes bind shards to a manifest generation; TLS authenticates its publisher.
use crate::{files, invalid, tokens, Index, Page, Result, FORMAT_VERSION, MAX_PAGES};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};
use ylang::{Node, NodeKind, Version};

pub const MANIFEST_KEY: &str = "search-manifest";
pub const MAX_SHARDS: usize = 64;
pub const MAX_PUBLICATION_BYTES: usize = 6 * 1024 * 1024;
const CHUNK_BYTES: usize = 96 * 1024;
const HEX_LINE_BYTES: usize = 768;

#[derive(Clone, Debug)]
pub struct ShardRef {
    pub key: String,
    pub(crate) bytes: usize,
    pub digest: String,
}
#[derive(Clone, Debug)]
pub struct Manifest {
    pub(crate) pages: usize,
    pub(crate) bytes: usize,
    pub(crate) shards: Vec<ShardRef>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    page: Page,
    terms: Vec<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    version: u32,
    records: Vec<Record>,
}

pub struct Publication {
    manifest: String,
    /// Stable content-addressed document keys; no filenames or remote URLs.
    shards: Vec<(String, String)>,
}
impl Publication {
    pub fn manifest(&self) -> &str {
        &self.manifest
    }
    pub fn shards(&self) -> &[(String, String)] {
        &self.shards
    }
    pub fn build(index: &Index) -> Result<Self> {
        let records = index
            .pages()
            .iter()
            .map(|page| Record {
                page: page.clone(),
                terms: page_terms(page),
            })
            .collect();
        if index.pages().iter().any(|p| p.address.is_none()) {
            return Err(invalid(
                "publication requires supplied canonical website identities",
            ));
        }
        let payload = serde_json::to_vec(&Payload {
            version: FORMAT_VERSION,
            records,
        })
        .map_err(|e| invalid(e.to_string()))?;
        if payload.len() > MAX_PUBLICATION_BYTES {
            return Err(invalid("publication exceeds byte budget"));
        }
        let mut shards = Vec::new();
        let mut refs = Vec::new();
        for (i, chunk) in payload.chunks(CHUNK_BYTES).enumerate() {
            let mut doc = "@page 0.2\n  title Yami Search Shard\n:: search-shard 1\n".to_owned();
            for part in chunk.chunks(HEX_LINE_BYTES) {
                doc.push_str(&format!("  :: data {}\n", hex(part)));
            }
            validate_envelope(&doc, "Yami Search Shard", "search-shard 1")?;
            let digest = digest(doc.as_bytes());
            let key = format!("search-{i:04}-{digest}");
            refs.push(ShardRef {
                key: key.clone(),
                bytes: doc.len(),
                digest,
            });
            shards.push((key, doc));
        }
        let mut manifest = format!("@page 0.2\n  title Yami Search Manifest\n:: search-manifest 1\n  :: pages {}\n  :: bytes {}\n", index.pages().len(), payload.len());
        for entry in refs {
            manifest.push_str(&format!(
                "  :: shard {} {} {}\n",
                entry.key, entry.bytes, entry.digest
            ));
        }
        Manifest::parse(&manifest)?;
        Ok(Self { manifest, shards })
    }
    /// New output directory only: never overwrite another site or publication.
    pub fn export(&self, output: impl AsRef<Path>) -> Result<()> {
        let output = output.as_ref();
        files::check_path(output)?;
        fs::create_dir(output)?;
        let result = (|| {
            for (key, doc) in &self.shards {
                files::write(&output.join(format!("{key}.ylang")), doc)?;
            }
            files::write(
                &output.join(format!("{MANIFEST_KEY}.ylang")),
                &self.manifest,
            )
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(output);
        }
        result
    }
    pub fn load(directory: impl AsRef<Path>) -> Result<Index> {
        let directory = directory.as_ref();
        let manifest = Manifest::parse(&files::read(
            &directory.join(format!("{MANIFEST_KEY}.ylang")),
            ylang::MAX_SOURCE,
        )?)?;
        let mut shards = Vec::new();
        for entry in &manifest.shards {
            shards.push(files::read(
                &directory.join(format!("{}.ylang", entry.key)),
                ylang::MAX_SOURCE,
            )?);
        }
        manifest.assemble(&shards)
    }
}
impl Manifest {
    pub fn page_count(&self) -> usize {
        self.pages
    }
    pub fn payload_bytes(&self) -> usize {
        self.bytes
    }
    pub fn shards(&self) -> &[ShardRef] {
        &self.shards
    }
    pub fn parse(source: &str) -> Result<Self> {
        let nodes = validate_envelope(source, "Yami Search Manifest", "search-manifest 1")?;
        if nodes.len() < 3 || nodes.len() > MAX_SHARDS + 2 {
            return Err(invalid("manifest shard count outside budget"));
        }
        let pages = number(
            leaf(&nodes[0])?
                .strip_prefix("pages ")
                .ok_or_else(|| invalid("missing page count"))?,
        )?;
        let bytes = number(
            leaf(&nodes[1])?
                .strip_prefix("bytes ")
                .ok_or_else(|| invalid("missing byte count"))?,
        )?;
        if pages > MAX_PAGES || bytes == 0 || bytes > MAX_PUBLICATION_BYTES {
            return Err(invalid("manifest exceeds budgets"));
        }
        let mut shards = Vec::new();
        let mut keys = BTreeSet::new();
        for (i, node) in nodes[2..].iter().enumerate() {
            let fields: Vec<_> = leaf(node)?.split(' ').collect();
            if fields.len() != 4
                || fields[0] != "shard"
                || !ylang::valid_key(fields[1])
                || fields[3].len() != 64
                || !fields[3]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
                || fields[1] != format!("search-{i:04}-{}", fields[3])
                || !keys.insert(fields[1].to_owned())
            {
                return Err(invalid(
                    "invalid, duplicate or out-of-order shard reference",
                ));
            }
            let size = number(fields[2])?;
            if size == 0 || size > ylang::MAX_SOURCE {
                return Err(invalid("invalid shard byte count"));
            }
            shards.push(ShardRef {
                key: fields[1].into(),
                bytes: size,
                digest: fields[3].into(),
            });
        }
        if shards.len() != bytes.div_ceil(CHUNK_BYTES) {
            return Err(invalid("inconsistent payload/shard count"));
        }
        Ok(Self {
            pages,
            bytes,
            shards,
        })
    }
    pub fn validate_shard(&self, position: usize, source: &str) -> Result<Vec<u8>> {
        let entry = self
            .shards
            .get(position)
            .ok_or_else(|| invalid("unexpected shard"))?;
        if source.len() != entry.bytes || digest(source.as_bytes()) != entry.digest {
            return Err(invalid("shard size/hash differs from manifest"));
        }
        let nodes = validate_envelope(source, "Yami Search Shard", "search-shard 1")?;
        if nodes.is_empty() || nodes.len() > CHUNK_BYTES.div_ceil(HEX_LINE_BYTES) {
            return Err(invalid("shard node budget exceeded"));
        }
        let mut bytes = Vec::new();
        for (i, node) in nodes.iter().enumerate() {
            let data = leaf(node)?
                .strip_prefix("data ")
                .ok_or_else(|| invalid("expected hex data leaf"))?;
            if data.is_empty()
                || data.len() > HEX_LINE_BYTES * 2
                || (i + 1 < nodes.len() && data.len() != HEX_LINE_BYTES * 2)
            {
                return Err(invalid("invalid hex chunk size"));
            }
            bytes.extend(unhex(data)?);
        }
        let expected = if position + 1 < self.shards.len() {
            CHUNK_BYTES
        } else {
            self.bytes - position * CHUNK_BYTES
        };
        if bytes.len() != expected {
            return Err(invalid("inconsistent decoded shard length"));
        }
        Ok(bytes)
    }
    pub fn assemble(&self, sources: &[String]) -> Result<Index> {
        if sources.len() != self.shards.len() {
            return Err(invalid("incomplete shard transfer"));
        }
        let mut payload = Vec::with_capacity(self.bytes);
        for (i, source) in sources.iter().enumerate() {
            payload.extend(self.validate_shard(i, source)?);
        }
        self.decode(&payload)
    }
    pub(crate) fn decode(&self, bytes: &[u8]) -> Result<Index> {
        if bytes.len() != self.bytes {
            return Err(invalid("incomplete payload"));
        }
        let payload: Payload = serde_json::from_slice(bytes)
            .map_err(|e| invalid(format!("invalid publication: {e}")))?;
        if payload.version != FORMAT_VERSION || payload.records.len() != self.pages {
            return Err(invalid(
                "unsupported payload version or inconsistent page count",
            ));
        }
        let mut pages = Vec::new();
        let mut previous = None;
        for record in payload.records {
            record.page.validate()?;
            let id = record
                .page
                .address
                .clone()
                .ok_or_else(|| invalid("published page lacks canonical address"))?;
            if previous.as_ref().is_some_and(|p| p >= &id)
                || record.terms != page_terms(&record.page)
            {
                return Err(invalid(
                    "duplicate/out-of-order pages or inconsistent normalized terms",
                ));
            }
            previous = Some(id);
            pages.push(record.page);
        }
        Index::from_pages(pages)
    }
}

fn page_terms(page: &Page) -> Vec<String> {
    [&page.title, &page.headings, &page.text]
        .into_iter()
        .flat_map(|s| tokens(s))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn validate_envelope(source: &str, title: &str, wrapper: &str) -> Result<Vec<Node>> {
    let doc = ylang::parse(source).map_err(|e| invalid(e.to_string()))?;
    if doc.version != Version::V02
        || doc.metadata.title != title
        || doc.metadata.language.is_some()
        || doc.nodes.len() != 1
    {
        return Err(invalid("invalid search YLang envelope"));
    }
    match doc.nodes.into_iter().next().map(|n| n.kind) {
        Some(NodeKind::Section { name, children }) if name == wrapper => Ok(children),
        _ => Err(invalid("unsupported search format/version")),
    }
}
fn leaf(node: &Node) -> Result<&str> {
    match &node.kind {
        NodeKind::Section { name, children } if children.is_empty() => Ok(name),
        _ => Err(invalid("expected inert section leaf")),
    }
}
fn number(text: &str) -> Result<usize> {
    let value = text
        .parse::<usize>()
        .map_err(|_| invalid("invalid unsigned size"))?;
    if value.to_string() != text {
        return Err(invalid("noncanonical size"));
    }
    Ok(value)
}
fn digest(bytes: &[u8]) -> String {
    hex(&yami_identity::sha256(bytes))
}
fn hex(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|b| {
            [
                ALPHABET[(b >> 4) as usize] as char,
                ALPHABET[(b & 15) as usize] as char,
            ]
        })
        .collect()
}
fn unhex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
    {
        return Err(invalid("invalid lowercase hex"));
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).map_err(|_| invalid("invalid hex"))?;
            u8::from_str_radix(s, 16).map_err(|_| invalid("invalid hex"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(records: &str) -> Vec<u8> {
        format!("{{\"version\":1,\"records\":[{records}]}}").into_bytes()
    }
    fn record() -> String {
        r#"{"page":{"key":"home","title":"Café","headings":"","text":"Original","address":"yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home"},"terms":["cafe","original"]}"#.into()
    }
    fn decode(data: &[u8], count: usize) -> Result<Index> {
        Manifest {
            pages: count,
            bytes: data.len(),
            shards: Vec::new(),
        }
        .decode(data)
    }
    #[test]
    fn payload_rejects_inconsistent_terms_duplicates_unknown_fields_and_versions() {
        let good = record();
        assert!(decode(&payload(&good), 1).is_ok());
        for bad in [
            good.replace("original\"]", "forged\"]"),
            good.replace("\"key\":", "\"unknown\":"),
            good.replace("Café", ""),
            good.replace("/home", "/other"),
        ] {
            assert!(decode(&payload(&bad), 1).is_err());
        }
        assert!(decode(&payload(&format!("{good},{good}")), 2).is_err());
        let bad_version = String::from_utf8(payload(&good))
            .unwrap()
            .replace("\"version\":1", "\"version\":2");
        assert!(decode(bad_version.as_bytes(), 1).is_err());
        assert!(decode(&payload(&good), 2).is_err());
    }
}
