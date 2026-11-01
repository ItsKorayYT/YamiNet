use std::{path::PathBuf, sync::Arc};
use yami_games::*;
fn sample() -> (Manifest, Vec<u8>) {
    load_local(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../games/lantern-drift/game.yami-game.json"),
    )
    .unwrap()
}
fn module(imports: &str, update: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
    (import "yami_game_v1" "clear" (func $clear (param i32)))
    {imports}
    (func (export "yami_game_abi") (result i32) i32.const 2)
    (func (export "yami_game_init"))
    (func (export "yami_game_input") (param i32 i32))
    (func (export "yami_game_update") (param i32) i32.const 255 call $clear {update})
    (func (export "yami_game_reset")))"#
    ))
    .unwrap()
}
#[test]
fn abi2_negotiates_exactly_and_plays_the_actual_rust_game() {
    let (manifest, bytes) = sample();
    assert!(Runtime::with_assets(&bytes, Some(1), manifest.prepared.clone()).is_err());
    let mut vm = Runtime::with_assets(&bytes, Some(2), manifest.prepared).unwrap();
    let screen = vm.frame(0, 0).unwrap();
    assert!(screen
        .commands
        .iter()
        .any(|c| matches!(c, Command::Text{text,..} if text=="LANTERN DRIFT")));
    assert!(!screen.assets.texture(0).unwrap().data().is_empty());
    let started = vm
        .frame_input(Input {
            pressed: START,
            x: 320,
            y: 320,
            ..Input::default()
        })
        .unwrap();
    assert!(started
        .commands
        .iter()
        .any(|c| matches!(c, Command::Sprite { .. })));
    assert!(vm
        .take_audio()
        .iter()
        .any(|c| matches!(c, AudioCommand::Play { looping: true, .. })));
    // Actually execute animated gameplay, mouse steering and pulse/score mechanics.
    let mut scored = false;
    for tick in 0..300 {
        let frame = vm
            .frame_input(Input {
                held: MOUSE_LEFT,
                pressed: if tick % 90 == 0 { START } else { 0 },
                x: 70,
                y: 140,
                ..Input::default()
            })
            .unwrap();
        if frame.commands.iter().any(|c| matches!(c,Command::Text{text,..} if text.starts_with("LIGHT ") && !text.starts_with("LIGHT 00"))) { scored=true; }
    }
    assert!(scored);
    vm.frame_input(Input {
        pressed: SECONDARY,
        ..Input::default()
    })
    .unwrap();
    assert!(vm.take_audio().contains(&AudioCommand::Mute(true)));
    vm.reset().unwrap();
    assert_eq!(vm.frame(0, 0).unwrap().commands, screen.commands);
}
#[test]
fn sprite_references_rectangles_scales_and_layers_fail_closed() {
    let (manifest, _) = sample();
    let import = r#"(import "yami_game_v2" "sprite" (func $s (param i32 i32 i32 i32 i32 i32 i32 i32 i32 i32)))"#;
    for args in [
        "9 0 0 32 32 0 0 32 32 0",
        "0 -1 0 32 32 0 0 32 32 0",
        "0 120 0 32 32 0 0 32 32 0",
        "0 0 0 32 32 0 0 2147483647 32 0",
        "0 0 0 32 32 0 0 32 32 128",
    ] {
        let body = args
            .split_whitespace()
            .map(|n| format!("i32.const {n} "))
            .collect::<String>()
            + "call $s";
        assert!(
            Runtime::with_assets(&module(import, &body), Some(2), manifest.prepared.clone())
                .unwrap()
                .frame(0, 0)
                .is_err(),
            "{args}"
        );
    }
    let command = |layer| {
        format!("i32.const 0 i32.const 0 i32.const 0 i32.const 32 i32.const 32 i32.const -16 i32.const -16 i32.const 64 i32.const 64 i32.const {layer} call $s ")
    };
    let mut vm = Runtime::with_assets(
        &module(import, &(command(2) + &command(-1) + &command(2))),
        Some(2),
        manifest.prepared,
    )
    .unwrap();
    let frame = vm.frame(0, 0).unwrap();
    assert!(matches!(
        frame.commands[1],
        Command::Sprite { layer: -1, .. }
    ));
    assert!(matches!(
        frame.commands[2],
        Command::Sprite { layer: 2, .. }
    ));
}
#[test]
fn input_press_release_and_game_coordinates_reach_guest() {
    let import = r#"(import "yami_game_v2" "input" (func $i (param i32) (result i32)))"#;
    let body="i32.const 0 call $i i32.const 528 i32.ne if unreachable end i32.const 1 call $i i32.const 256 i32.ne if unreachable end i32.const 2 call $i i32.const 32 i32.ne if unreachable end i32.const 3 call $i i32.const 639 i32.ne if unreachable end i32.const 4 call $i i32.const 479 i32.ne if unreachable end";
    let mut vm = Runtime::new(&module(import, body)).unwrap();
    vm.frame_input(Input {
        held: UP | MOUSE_RIGHT,
        pressed: MOUSE_LEFT,
        released: DOWN,
        x: 639,
        y: 479,
    })
    .unwrap();
    let bad = module(import, "i32.const 99 call $i drop");
    assert!(Runtime::new(&bad).unwrap().frame(0, 0).is_err());
}
#[test]
fn audio_capability_validates_ids_channels_values_and_command_budget() {
    let (manifest, _) = sample();
    let import = r#"(import "yami_game_v2" "audio" (func $a (param i32 i32 i32 i32 i32)))"#;
    for args in [
        "0 8 1 500 0",
        "0 0 0 500 0",
        "0 0 1 1001 0",
        "0 0 1 500 2",
        "4 0 1 500 0",
        "3 0 1 2 0",
    ] {
        let body = args
            .split_whitespace()
            .map(|n| format!("i32.const {n} "))
            .collect::<String>()
            + "call $a";
        assert!(
            Runtime::with_assets(&module(import, &body), Some(2), manifest.prepared.clone())
                .unwrap()
                .frame(0, 0)
                .is_err()
        );
    }
    let body = "i32.const 1 i32.const 0 i32.const 0 i32.const 0 i32.const 0 call $a ".repeat(33);
    assert!(
        Runtime::with_assets(&module(import, &body), Some(2), manifest.prepared)
            .unwrap()
            .frame(0, 0)
            .is_err()
    );
    // Valid commands still work in a headless VM without an audio device.
    let mut vm = Runtime::new(&module(
        import,
        "i32.const 3 i32.const 0 i32.const 0 i32.const 1 i32.const 0 call $a",
    ))
    .unwrap();
    vm.frame(0, 0).unwrap();
    assert_eq!(vm.take_audio(), vec![AudioCommand::Mute(true)]);
}
fn resource(id: u32, kind: AssetKind, data: &[u8]) -> AssetResource {
    use sha2::{Digest, Sha256};
    AssetResource {
        id,
        key: format!("asset-{id}"),
        kind,
        bytes: data.len(),
        sha256: Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    }
}
#[test]
fn images_and_audio_are_bounded_typed_integrity_checked_and_cached() {
    let png = include_bytes!("../../../games/lantern-drift/sprites.png");
    let wav = include_bytes!("../../../games/lantern-drift/chime.wav");
    let mut assets = Assets::default();
    assets
        .insert(&resource(0, AssetKind::Png, png), png)
        .unwrap();
    let address = assets.texture(0).unwrap().data().as_ptr();
    assert_eq!(assets.texture(0).unwrap().data().as_ptr(), address);
    assert!(assets
        .insert(&resource(0, AssetKind::Png, png), png)
        .is_err());
    assets
        .insert(&resource(1, AssetKind::Wav, wav), wav)
        .unwrap();
    assert!(assets
        .insert(&resource(2, AssetKind::Wav, png), png)
        .is_err());
    assert!(assets
        .insert(&resource(2, AssetKind::Png, wav), wav)
        .is_err());
    let mut corrupt = png.to_vec();
    corrupt[45] ^= 1;
    assert!(assets
        .insert(&resource(2, AssetKind::Png, png), &corrupt)
        .unwrap_err()
        .contains("integrity"));
    for invalid in [
        b"not PNG".to_vec(),
        png[..png.len() - 4].to_vec(),
        vec![0; 256 * 1024 + 1],
    ] {
        assert!(assets
            .insert(&resource(2, AssetKind::Png, &invalid), &invalid)
            .is_err());
    }
    let big = yami_render::assets::encode_png(1024, 1024, &vec![255; 4 * 1024 * 1024]).unwrap();
    let mut assets = Assets::default();
    for id in 0..4 {
        assets
            .insert(&resource(id, AssetKind::Png, &big), &big)
            .unwrap();
    }
    assert!(assets
        .insert(&resource(4, AssetKind::Png, &big), &big)
        .is_err());
    // Header claims a huge image: reject before allocating a decoded frame.
    let mut huge = png.to_vec();
    huge[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Assets::default()
        .insert(&resource(0, AssetKind::Png, &huge), &huge)
        .is_err());
    let mut long = wav.to_vec();
    long[24..28].copy_from_slice(&1_u32.to_le_bytes());
    assert!(Assets::default()
        .insert(&resource(0, AssetKind::Wav, &long), &long)
        .is_err());
}
#[test]
fn hosted_v2_rejects_untrusted_asset_declarations() {
    let (manifest, bytes) = sample();
    let value = serde_json::json!({"version":2,"abi":2,"title":"Test","description":"","module":{"key":"game","bytes":bytes.len(),"sha256":"a".repeat(64)},"assets":manifest.assets});
    HostedManifest::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
    for (field, bad) in [
        ("key", serde_json::json!("yami://other/image")),
        ("key", serde_json::json!("../escape")),
        ("kind", serde_json::json!("url")),
        ("bytes", serde_json::json!(0)),
        ("sha256", serde_json::json!("0")),
        ("id", serde_json::json!(16)),
        ("url", serde_json::json!("https://evil.invalid")),
    ] {
        let mut bad_value = value.clone();
        bad_value["assets"][0][field] = bad;
        assert!(
            HostedManifest::parse(&serde_json::to_vec(&bad_value).unwrap()).is_err(),
            "{field}"
        );
    }
    let mut bad = value.clone();
    bad["assets"][1]["id"] = serde_json::json!(0);
    assert!(HostedManifest::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
    assert!(Runtime::with_assets(&bytes, Some(2), Arc::default())
        .unwrap()
        .frame(0, 0)
        .is_err());
}

#[test]
fn lantern_drift_has_reachable_win_and_lose_screens_in_actual_wasm() {
    let (manifest, bytes) = sample();
    let mut vm = Runtime::with_assets(&bytes, Some(2), manifest.prepared).unwrap();
    let mut frame = vm.frame(0, START).unwrap();
    let (mut px, mut py) = (320, 320);
    let mut won = false;
    for tick in 0..3000 {
        for c in &frame.commands {
            if let Command::Sprite { sy: 0, x, y, .. } = c {
                px = x + 32;
                py = y + 32;
            }
        }
        let target = frame
            .commands
            .iter()
            .filter_map(|c| match c {
                Command::Sprite { sy: 32, x, y, .. } => Some((x + 20, y + 20)),
                _ => None,
            })
            .min_by_key(|(x, y)| (x - px).pow(2) + (y - py).pow(2));
        let (tx, ty) = target.unwrap_or((320, 240));
        let (mut dx, mut dy) = ((tx - px) as f32, (ty - py) as f32);
        let len = (dx * dx + dy * dy).sqrt().max(1.);
        dx /= len;
        dy /= len;
        for c in &frame.commands {
            if let Command::Sprite { sy: 64, x, y, .. } = c {
                let (mx, my) = (px - (x + 24), py - (y + 24));
                let dist = ((mx * mx + my * my) as f32).sqrt().max(1.);
                if dist < 85. {
                    dx += mx as f32 / dist * (85. - dist) / 18.;
                    dy += my as f32 / dist * (85. - dist) / 18.;
                }
            }
        }
        frame = vm
            .frame_input(Input {
                held: MOUSE_LEFT,
                pressed: if tick % 90 == 0 { ACTION } else { 0 },
                x: (px + (dx * 100.) as i32).clamp(0, 639),
                y: (py + (dy * 100.) as i32).clamp(94, 418),
                ..Input::default()
            })
            .unwrap();
        if frame
            .commands
            .iter()
            .any(|c| matches!(c,Command::Text{text,..} if text=="THE MARSH IS ALIGHT"))
        {
            won = true;
            break;
        }
        if frame
            .commands
            .iter()
            .any(|c| matches!(c,Command::Text{text,..} if text=="THE LIGHT FADES"))
        {
            break;
        }
    }
    assert!(
        won,
        "steered game should be winnable: {:?}",
        frame
            .commands
            .iter()
            .filter(|c| matches!(c, Command::Text { .. }))
            .collect::<Vec<_>>()
    );
    vm.reset().unwrap();
    vm.frame(0, START).unwrap();
    let mut lost = false;
    for _ in 0..3001 {
        let frame = vm.frame(0, 0).unwrap();
        if frame
            .commands
            .iter()
            .any(|c| matches!(c,Command::Text{text,..} if text=="THE LIGHT FADES"))
        {
            lost = true;
            break;
        }
    }
    assert!(lost, "idle game must end through hazards or the timer");
}
