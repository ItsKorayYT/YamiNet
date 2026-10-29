//! Signed directory sites: `<root>/site.sig` attributes the exact set of pages
//! and the shared style to a node identity. The signature file itself is
//! never served; YNP frames are unchanged and carry no signatures.
use crate::{site::MAX_LISTED, Site};
use std::{
    fs::{self, File},
    io::{self, Read},
};
use yami_identity::{sha256, NodeId, NodeIdentity, Result, SignatureFile, SiteManifest};

pub const SIGNATURE_FILE: &str = "site.sig";

/// Hash every page and the style exactly as the site would serve them now.
pub fn site_manifest(site: &Site) -> Result<SiteManifest> {
    let keys = site.pages()?;
    // `pages` stops listing at MAX_LISTED; a manifest must not omit pages.
    if keys.len() >= MAX_LISTED {
        return Err(io::Error::other(format!(
            "sites with {MAX_LISTED} or more pages cannot be signed"
        ))
        .into());
    }
    let changed = || io::Error::other("site changed while it was being read; try again");
    let style = sha256(site.style()?.as_bytes());
    let mut pages = Vec::with_capacity(keys.len());
    for key in keys {
        let document = site.lookup(&key)?.ok_or_else(changed)?;
        if sha256(document.style.as_bytes()) != style {
            return Err(changed().into());
        }
        pages.push((key, sha256(document.source.as_bytes())));
    }
    SiteManifest::new(style, pages)
}

/// Sign the site's current manifest and write `<root>/site.sig`, replacing
/// any earlier signature. Re-sign after every change to the site.
pub fn sign_site(site: &Site, identity: &NodeIdentity) -> Result<SignatureFile> {
    let signed = SignatureFile::sign(identity, &site_manifest(site)?);
    fs::write(site.root().join(SIGNATURE_FILE), signed.to_string())?;
    Ok(signed)
}

/// Succeeds only if `site.sig` was made by `expected` over the site exactly
/// as it is now: any edited, added or removed page or style fails.
pub fn verify_site(site: &Site, expected: &NodeId) -> Result<()> {
    let path = site.root().join(SIGNATURE_FILE);
    let file = File::open(&path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} not found; the site is unsigned", path.display()),
        ),
        _ => error,
    })?;
    let mut text = String::new();
    file.take(SignatureFile::MAX_LEN as u64 + 1)
        .read_to_string(&mut text)?;
    SignatureFile::parse(&text)?.verify(&site_manifest(site)?, expected)
}
