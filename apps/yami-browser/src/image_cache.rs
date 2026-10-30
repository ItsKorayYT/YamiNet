//! Per-loader, bounded RAM decode cache. Every request still authenticates and
//! fetches current encoded bytes. Exact owner + resource + bytes prevent stale
//! content and identity confusion without retaining documents on disk.
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex},
};
use yami_identity::NodeId;
use yami_render::assets::{self, Image};
const MAX_ENTRIES: usize = 16;
const MAX_BYTES: usize = 16 * 1024 * 1024;
struct Entry {
    owner: NodeId,
    key: String,
    encoded: Vec<u8>,
    image: Arc<Image>,
}
impl Entry {
    fn bytes(&self) -> usize {
        self.encoded.len() + self.image.pixels.len()
    }
}
#[derive(Default)]
pub struct DecodeCache {
    entries: Mutex<VecDeque<Entry>>,
}
impl DecodeCache {
    pub fn decode(
        &self,
        owner: NodeId,
        key: &str,
        encoded: &[u8],
        budget: usize,
    ) -> io::Result<Arc<Image>> {
        ynp::validate_asset_key(key)?;
        {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(index) = entries
                .iter()
                .position(|e| e.owner == owner && e.key == key && e.encoded == encoded)
            {
                if entries[index].image.pixels.len() > budget {
                    return Err(io::Error::other("page decode budget exhausted"));
                }
                let entry = entries.remove(index).unwrap();
                let image = entry.image.clone();
                entries.push_front(entry);
                return Ok(image);
            }
        }
        // Decoding never holds the mutex or runs on the desktop thread.
        let image = Arc::new(assets::decode_png(encoded, budget)?);
        let entry = Entry {
            owner,
            key: key.into(),
            encoded: encoded.into(),
            image: image.clone(),
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        while !entries.is_empty()
            && (entries.len() >= MAX_ENTRIES
                || entries.iter().map(Entry::bytes).sum::<usize>() + entry.bytes() > MAX_BYTES)
        {
            entries.pop_back();
        }
        entries.push_front(entry);
        Ok(image)
    }
    pub fn retained_bytes(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(Entry::bytes)
            .sum()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_is_owner_resource_content_and_budget_scoped_and_evicts() {
        let one = *yami_identity::NodeIdentity::generate().unwrap().id();
        let two = *yami_identity::NodeIdentity::generate().unwrap().id();
        let png = include_bytes!("../../../sites/showcase/assets/cover.png");
        let cache = DecodeCache::default();
        let first = cache
            .decode(one, "cover", png, assets::MAX_DECODED)
            .unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &cache
                .decode(one, "cover", png, assets::MAX_DECODED)
                .unwrap()
        ));
        assert!(!Arc::ptr_eq(
            &first,
            &cache
                .decode(two, "cover", png, assets::MAX_DECODED)
                .unwrap()
        ));
        assert!(cache.decode(one, "cover", png, 1).is_err());
        let weak = Arc::downgrade(&first);
        drop(first);
        for i in 0..24 {
            cache
                .decode(one, &format!("asset{i}"), png, assets::MAX_DECODED)
                .unwrap();
        }
        assert!(weak.upgrade().is_none());
        assert!(cache.retained_bytes() <= MAX_BYTES);
        let mut corrupt = png.to_vec();
        corrupt[0] = 0;
        assert!(cache
            .decode(one, "cover", &corrupt, assets::MAX_DECODED)
            .is_err());
    }
}
