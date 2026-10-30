//! Static, bounded site authoring using the production parsers and Site reader.
use crate::Site;
use std::{collections::BTreeSet, fs, io, path::Path};
use ylang::{Node, NodeKind};
const MAX_PAGES: usize = 256;
const MAX_ASSETS: usize = 256;
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
#[derive(Debug)]
pub struct Report {
    pub pages: Vec<String>,
    pub assets: Vec<String>,
    pub bytes: usize,
}
fn inventory(dir: &Path, extension: &str, max: usize) -> io::Result<Vec<String>> {
    let mut keys = Vec::new();
    for entry in fs::read_dir(dir)? {
        let e = entry?;
        let name = e.file_name();
        let Some(name) = name.to_str() else {
            return Err(invalid("site filenames must be Unicode"));
        };
        if let Some(key) = name.strip_suffix(extension) {
            if !e.file_type()?.is_file() {
                return Err(invalid(format!(
                    "{name}: expected a regular file, never a symlink"
                )));
            }
            if extension == ".png" {
                ynp::validate_asset_key(key)?;
            } else {
                ynp::validate_key(key)?;
            }
            keys.push(key.into());
            if keys.len() > max {
                return Err(invalid(
                    "site inventory limit exceeded (256 pages / 256 assets)",
                ));
            }
        }
    }
    keys.sort();
    Ok(keys)
}
pub fn check(root: impl AsRef<Path>) -> io::Result<Report> {
    let site = Site::open(root)?;
    let pages = inventory(site.root(), ".ylang", MAX_PAGES)?;
    if pages.is_empty() {
        return Err(invalid("site has no .ylang pages"));
    }
    let style = site.style()?;
    if !style.is_empty() {
        ysl::parse(&style).map_err(|e| invalid(format!("default.ysl: {e}")))?;
    }
    let asset_dir = site.root().join("assets");
    let assets = match fs::symlink_metadata(&asset_dir) {
        Ok(m) if m.file_type().is_dir() => inventory(&asset_dir, ".png", MAX_ASSETS)?,
        Ok(_) => {
            return Err(invalid(
                "assets must be a regular directory, never a symlink",
            ))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e),
    };
    let mut sizes = std::collections::BTreeMap::new();
    let mut bytes = style.len();
    for key in &assets {
        let data = site
            .asset(key)?
            .ok_or_else(|| invalid(format!("missing asset {key}")))?;
        let image = yami_render::assets::decode_png(&data, yami_render::assets::MAX_DECODED)
            .map_err(|e| invalid(format!("assets/{key}.png: {e}")))?;
        bytes += data.len();
        sizes.insert(key.clone(), (data.len(), image.pixels.len()));
    }
    let page_set: BTreeSet<_> = pages.iter().cloned().collect();
    for key in &pages {
        let data = site
            .lookup(key)?
            .ok_or_else(|| invalid(format!("{key}.ylang cannot be served safely")))?;
        bytes += data.source.len();
        let doc = ylang::parse(&data.source).map_err(|e| invalid(format!("{key}.ylang: {e}")))?;
        let mut images = BTreeSet::new();
        validate_nodes(&doc.nodes, &page_set, &mut images, 0)
            .map_err(|e| invalid(format!("{key}.ylang: {e}")))?;
        if images.len() > yami_render::assets::MAX_PAGE_IMAGES {
            return Err(invalid(format!(
                "{key}: at most 8 distinct images per page"
            )));
        }
        let (mut compressed, mut decoded) = (0, 0);
        for image in images {
            let (c, d) = sizes
                .get(&image)
                .ok_or_else(|| invalid(format!("{key}.ylang: missing assets/{image}.png")))?;
            compressed += c;
            decoded += d;
        }
        // Include framing overhead so the generated site fits the real loader budget.
        if compressed + 8 * 128 > yami_render::assets::MAX_PAGE_COMPRESSED
            || decoded > yami_render::assets::MAX_PAGE_DECODED
        {
            return Err(invalid(format!("{key}: page image budget exceeded")));
        }
    }
    Ok(Report {
        pages,
        assets,
        bytes,
    })
}
fn validate_nodes(
    nodes: &[Node],
    pages: &BTreeSet<String>,
    images: &mut BTreeSet<String>,
    depth: usize,
) -> io::Result<()> {
    if depth > 32 {
        return Err(invalid("document nesting limit exceeded"));
    }
    for node in nodes {
        match &node.kind {
            NodeKind::Image { key, .. } => {
                images.insert(key.clone());
            }
            NodeKind::Link { target, .. } => {
                if let Some(key) = target.strip_prefix('/') {
                    if !pages.contains(key) {
                        return Err(invalid(format!(
                            "line {}: local link /{key} has no page",
                            node.span.line
                        )));
                    }
                } else if let Some(rest) = target.strip_prefix("yami://") {
                    let (id, key) = rest
                        .split_once('/')
                        .ok_or_else(|| invalid("identity link requires /page"))?;
                    let _: yami_identity::NodeId = id
                        .parse()
                        .map_err(|_| invalid("invalid full destination identity in link"))?;
                    ynp::validate_key(key)?;
                } else if target.starts_with("ynps://") {
                    ynp_tls::parse_url(target)?;
                } else if let Some(rest) = target.strip_prefix("ynp://") {
                    // Legacy identity targets remain supported by the browser.
                    let (host, key) = rest
                        .split_once('/')
                        .ok_or_else(|| invalid("invalid legacy link"))?;
                    if host.starts_with("yn1") {
                        let _: yami_identity::NodeId = host
                            .parse()
                            .map_err(|_| invalid("invalid legacy identity link"))?;
                        ynp::validate_key(key)?;
                    } else {
                        ynp::parse_url(target)?;
                    }
                } else {
                    return Err(invalid("unsupported link target"));
                }
            }
            _ => {}
        }
        validate_nodes(node.kind.children(), pages, images, depth + 1)?;
    }
    Ok(())
}
pub fn new_site(root: impl AsRef<Path>, template: &str) -> io::Result<Report> {
    let root = root.as_ref();
    let (title, intro, section) = match template {
        "personal" => (
            "My Yami home",
            "Welcome to my own corner of YamiNet.",
            "About me",
        ),
        "blog" => (
            "Field notes",
            "Thoughts, observations and longer articles.",
            "First article",
        ),
        "portfolio" => (
            "Studio portfolio",
            "Independent projects, made with care.",
            "Selected project",
        ),
        _ => return Err(invalid("template must be personal, blog or portfolio")),
    };
    // create_dir deliberately refuses existing destinations, including private keys.
    fs::create_dir(root)?;
    let result = (|| {
        fs::create_dir(root.join("assets"))?;
        fs::write(root.join("default.ysl"), STYLE)?;
        let source=format!("@page 0.2\n  title {title}\n= {title}\n{intro} This is [strong: my website], written in [code: YLang 0.2].\nText can wrap naturally across\nadjacent source lines, including Unicode: café · 日本語.\n\n! cover | Abstract mountains beneath a warm sunset\n:: card {section}\n  A quiet place for [em: original ideas] and native documents.\n  :: list\n    - Write something useful\n    - Keep your identity across updates\n      :: steps\n        + Check your site\n        + Publish using your existing node\n  > /about | Read more\n---\n:: row Explore\n  :: card Notes\n    Pages stay inert and readable.\n  :: card Projects\n    Images are served by this node through authenticated YNP.\n> /about | About this site\n");
        fs::write(root.join("home.ylang"), source)?;
        fs::write(root.join("about.ylang"),format!("@page 0.2\n  title {section}\n= {section}\nThis page belongs to the same stable cryptographic identity.\n:: card Publishing\n  Update these files without replacing your identity directory.\n  :: steps\n    + Edit a page\n    + Run site check\n    + Restart or reload the site\n> /home | Return home\n"))?;
        let (width, height) = (240u32, 140u32);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let mountain = y > 95 - (x as i32 - 100).unsigned_abs().min(70) / 2;
                let rgba = if mountain {
                    [36, 72, 85, 255]
                } else {
                    [
                        220 - (y / 3) as u8,
                        130 + (x / 5) as u8,
                        110 + (y / 2) as u8,
                        255,
                    ]
                };
                pixels.extend_from_slice(&rgba);
            }
        }
        let png = yami_render::assets::encode_png(width, height, &pixels)?;
        fs::write(root.join("assets/cover.png"), png)?;
        check(root)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(root);
    }
    result
}
/// A build exports only validated pages, shared style and PNGs; never keys/state.
pub fn build(source: impl AsRef<Path>, output: impl AsRef<Path>) -> io::Result<Report> {
    let report = check(&source)?;
    let site = Site::open(source)?;
    let output = output.as_ref();
    let parent = fs::canonicalize(
        output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    if parent.starts_with(site.root()) {
        return Err(invalid("build output must be outside the source site"));
    }
    fs::create_dir(output)?;
    let result = (|| {
        fs::create_dir(output.join("assets"))?;
        fs::write(output.join("default.ysl"), site.style()?)?;
        for key in &report.pages {
            let doc = site
                .lookup(key)?
                .ok_or_else(|| invalid("page disappeared during build"))?;
            fs::write(output.join(format!("{key}.ylang")), doc.source)?;
        }
        for key in &report.assets {
            let bytes = site
                .asset(key)?
                .ok_or_else(|| invalid("asset disappeared during build"))?;
            fs::write(output.join("assets").join(format!("{key}.png")), bytes)?;
        }
        check(output)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(output);
    }
    result
}
pub const STYLE:&str="@style 0.2\ncanvas\n  width 960\n  padding 24\n  paper #101820\ntext\n  ink #d8e2ec\n  size 18\n  line-height 155\nheading\n  ink #61dafb\n  size 40\n  weight 600\nsection\n  ink #b4d273\n  size 24\nlink\n  ink #ffa970\n  hover #ffd5a0\n  active #61dafb\ncard\n  paper #192531\n  ink #d8e2ec\n  border #405970\n  border-width 1\n  radius 14\n  padding 20\n  gap 20\nrow\n  gap 20\n  min-width 240\nimage\n  width 800\n  height 500\n  align center\n  gap 16\n";
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_validate_export_and_refuse_overwrite() {
        let root = std::env::temp_dir().join(format!("yami-authoring-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        for name in ["personal", "blog", "portfolio"] {
            let dir = root.join(name);
            let report = new_site(&dir, name).unwrap();
            assert_eq!(report.pages, vec!["about", "home"]);
            assert_eq!(report.assets, vec!["cover"]);
            assert!(new_site(&dir, name).is_err());
            fs::write(dir.join("private.key"), "not for publication").unwrap();
            let out = root.join(format!("{name}-built"));
            build(&dir, &out).unwrap();
            assert!(!out.join("private.key").exists());
            assert!(build(&dir, &out).is_err());
            assert!(build(&dir, dir.join("build")).is_err());
            fs::write(
                dir.join("about.ylang"),
                "@page 0.2\n  title Bad\n> /missing | Missing\n",
            )
            .unwrap();
            assert!(check(&dir).unwrap_err().to_string().contains("missing"));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
