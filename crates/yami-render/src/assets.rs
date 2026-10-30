//! Bounded PNG decoding/preparation. All callers load/decode on worker threads.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Write},
    sync::Arc,
};
pub const MAX_COMPRESSED: usize = 256 * 1024;
pub const MAX_DIMENSION: u32 = 2048;
pub const MAX_PIXELS: usize = 1024 * 1024;
pub const MAX_DECODED: usize = MAX_PIXELS * 4;
pub const MAX_PAGE_IMAGES: usize = 8;
pub const MAX_PAGE_COMPRESSED: usize = 1024 * 1024;
pub const MAX_PAGE_DECODED: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}
impl Image {
    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_DIMENSION
            && self.height <= MAX_DIMENSION
            && (self.width as usize * self.height as usize) <= MAX_PIXELS
            && self.pixels.len() == self.width as usize * self.height as usize * 4
    }
}
#[derive(Debug, Clone)]
pub enum ImageState {
    Pending,
    Ready(Arc<Image>),
    Missing(String),
}
pub type Images = BTreeMap<String, ImageState>;
fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "PNG is invalid, animated, oversized or exceeds its decoding budget",
    )
}
pub fn decode_png(bytes: &[u8], budget: usize) -> io::Result<Image> {
    if bytes.len() > MAX_COMPRESSED
        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || !bytes.ends_with(b"\0\0\0\0IEND\xae\x42\x60\x82")
    {
        return Err(invalid());
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: 8 * 1024 * 1024,
    });
    decoder.set_ignore_text_chunk(true);
    decoder.set_ignore_iccp_chunk(true);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| invalid())?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(invalid)?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || pixels > MAX_PIXELS
        || pixels
            .checked_mul(4)
            .is_none_or(|bytes| bytes > budget.min(MAX_DECODED))
        || info.animation_control.is_some()
        || reader.output_buffer_size() > MAX_DECODED
    {
        return Err(invalid());
    }
    let mut raw = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut raw).map_err(|_| invalid())?;
    if frame.bit_depth != png::BitDepth::Eight {
        return Err(invalid());
    }
    raw.truncate(frame.buffer_size());
    reader.finish().map_err(|_| invalid())?;
    let channels = match frame.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        _ => return Err(invalid()),
    };
    if raw.len() != pixels * channels {
        return Err(invalid());
    }
    let mut output = Vec::with_capacity(pixels * 4);
    for pixel in raw.chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            4 => (pixel[0], pixel[1], pixel[2], pixel[3]),
            3 => (pixel[0], pixel[1], pixel[2], 255),
            2 => (pixel[0], pixel[0], pixel[0], pixel[1]),
            _ => (pixel[0], pixel[0], pixel[0], 255),
        };
        let premultiply = |v: u8| ((v as u16 * a as u16 + 127) / 255) as u8;
        output.extend([premultiply(r), premultiply(g), premultiply(b), a]);
    }
    Ok(Image {
        width,
        height,
        pixels: output.into(),
    })
}
/// Authoring/test helper. Input and compressed output are independently bounded.
pub fn encode_png(width: u32, height: u32, pixels: &[u8]) -> io::Result<Vec<u8>> {
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || width as usize * height as usize > MAX_PIXELS
        || pixels.len() != width as usize * height as usize * 4
    {
        return Err(invalid());
    }
    struct Output(Vec<u8>);
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.len() + bytes.len() > MAX_COMPRESSED {
                return Err(io::Error::other("encoded PNG exceeds 256 KiB"));
            }
            self.0.extend(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut output = Output(Vec::new());
    {
        let mut encoder = png::Encoder::new(&mut output, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(io::Error::other)?;
        writer.write_image_data(pixels).map_err(io::Error::other)?;
        writer.finish().map_err(io::Error::other)?;
    }
    Ok(output.0)
}
pub fn image_keys(document: &ylang::Document) -> Vec<String> {
    let mut keys = BTreeSet::new();
    let mut stack = vec![(document.nodes.as_slice(), 0)];
    let mut visited = 0;
    while let Some((nodes, depth)) = stack.pop() {
        if depth > ylang::MAX_DEPTH {
            continue;
        }
        for node in nodes {
            visited += 1;
            if visited > ylang::MAX_NODES {
                return keys.into_iter().collect();
            }
            if let ylang::NodeKind::Image { key, .. } = &node.kind {
                keys.insert(key.clone());
                if keys.len() == MAX_PAGE_IMAGES {
                    return keys.into_iter().collect();
                }
            }
            if !node.kind.children().is_empty() {
                stack.push((node.kind.children(), depth + 1));
            }
        }
    }
    keys.into_iter().collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_decoding_preserves_alpha_and_rejects_truncation_corruption_and_budgets() {
        let bytes = encode_png(2, 1, &[255, 0, 0, 128, 0, 255, 0, 255]).unwrap();
        let image = decode_png(&bytes, MAX_DECODED).unwrap();
        assert!(image.valid());
        assert_eq!(&*image.pixels, &[128, 0, 0, 128, 0, 255, 0, 255]);
        assert!(decode_png(&bytes, 7).is_err());
        for end in 0..bytes.len() {
            assert!(decode_png(&bytes[..end], MAX_DECODED).is_err());
        }
        let mut bad = bytes.clone();
        bad[29] ^= 1;
        assert!(decode_png(&bad, MAX_DECODED).is_err());
        assert!(decode_png(&vec![0; MAX_COMPRESSED + 1], MAX_DECODED).is_err());
        // IHDR dimensions are checked before output allocation, even if the
        // file's deflate data could describe something much smaller.
        let mut large = bytes;
        large[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode_png(&large, MAX_DECODED).is_err());
    }
    #[test]
    fn page_image_discovery_is_deduplicated_and_bounded() {
        let source = format!(
            "@page 0.2\n  title Assets\n{}",
            (0..30)
                .map(|n| format!("! image{n} | Alternative text\n"))
                .collect::<String>()
        );
        let document = ylang::parse(&source).unwrap();
        assert_eq!(image_keys(&document).len(), MAX_PAGE_IMAGES);
    }
}
