//! Repeatable local CPU benchmarks; no network or window required.
use std::{hint::black_box, time::Instant};
use yami_render::{
    assets::{self, ImageState, Images},
    native::{NativeDocument, TextEngine},
};
fn measure(name: &str, count: u32, mut run: impl FnMut()) {
    let start = Instant::now();
    for _ in 0..count {
        run();
    }
    println!(
        "{name}: {} us/iteration ({count} iterations)",
        start.elapsed().as_micros() / u128::from(count)
    );
}
fn main() {
    println!(
        "YamiNet {}; {} {}; release benchmark; viewport 1000x800",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let simple = ylang::parse("@page 0.2\n  title Simple\n= Hello\nA simple café page.\n").unwrap();
    let long = ylang::parse(&format!(
        "@page 0.2\n  title Long\n{}",
        "A representative paragraph of native document text.\n\n".repeat(600)
    ))
    .unwrap();
    let heavy = ylang::parse(&format!(
        "@page 0.2\n  title Images\n{}",
        (0..8)
            .map(|i| format!("! image{i} | Artwork\n"))
            .collect::<String>()
    ))
    .unwrap();
    let sheet = ysl::parse("@style 0.2").unwrap();
    let mut engine = TextEngine::new();
    measure("simple_layout", 100, || {
        black_box(NativeDocument::layout(&simple, &sheet, 1000.0, 1.0, &mut engine).unwrap());
    });
    measure("long_layout_600_paragraphs", 10, || {
        black_box(NativeDocument::layout(&long, &sheet, 1000.0, 1.0, &mut engine).unwrap());
    });
    let png = include_bytes!("../../../sites/showcase/assets/cover.png");
    measure("png_decode", 100, || {
        black_box(assets::decode_png(png, assets::MAX_DECODED).unwrap());
    });
    let cache = yami_browser::image_cache::DecodeCache::default();
    let owner = *yami_identity::NodeIdentity::generate().unwrap().id();
    cache
        .decode(owner, "cover", png, assets::MAX_DECODED)
        .unwrap();
    measure("png_cached_decode", 100, || {
        black_box(
            cache
                .decode(owner, "cover", png, assets::MAX_DECODED)
                .unwrap(),
        );
    });
    let image = cache
        .decode(owner, "cover", png, assets::MAX_DECODED)
        .unwrap();
    let mut images = Images::new();
    for i in 0..8 {
        images.insert(format!("image{i}"), ImageState::Ready(image.clone()));
    }
    measure("image_heavy_layout", 30, || {
        black_box(
            NativeDocument::layout_with_assets(&heavy, &sheet, 1000.0, 1.0, &mut engine, &images)
                .unwrap(),
        );
    });
    let layout =
        NativeDocument::layout_with_assets(&heavy, &sheet, 1000.0, 1.0, &mut engine, &images)
            .unwrap();
    let mut pixmap = tiny_skia::Pixmap::new(1000, 800).unwrap();
    measure("image_heavy_draw", 30, || {
        layout.draw(&mut pixmap, 0.0, 0.0, 0.0, 800.0, &mut engine);
        black_box(&pixmap);
    });
    let layout = NativeDocument::layout(&long, &sheet, 1000.0, 1.0, &mut engine).unwrap();
    measure("long_scrolled_draw", 30, || {
        layout.draw(&mut pixmap, 0.0, 0.0, 10000.0, 800.0, &mut engine);
        black_box(&pixmap);
    });
}
