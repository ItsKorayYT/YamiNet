use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use yami_games::*;
fn module(extra: &str, init: &str, input: &str, update: &str, reset: &str) -> Vec<u8> {
    wat::parse_str(format!(
        r#"(module
        (import "yami_game_v1" "clear" (func $clear (param i32)))
        {extra}
        (func (export "yami_game_abi") (result i32) i32.const 1)
        (func (export "yami_game_init") {init})
        (func (export "yami_game_input") (param i32 i32) {input})
        (func (export "yami_game_update") (param i32) i32.const 255 call $clear {update})
        (func (export "yami_game_reset") {reset}))"#
    ))
    .unwrap()
}
fn sample() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../games/breakout/game.yami-game.json")
}
fn has_text(frame: &Frame, needle: &str) -> bool {
    frame
        .commands
        .iter()
        .any(|c| matches!(c,Command::Text{text,..} if text.contains(needle)))
}
#[test]
fn invalid_modules_versions_exports_imports_and_start_fail_closed() {
    for bytes in [
        vec![],
        b"not wasm".to_vec(),
        vec![0; MAX_MODULE_BYTES + 1],
        wat::parse_str("(module)").unwrap(),
        module(
            "(import \"wasi_snapshot_preview1\" \"proc_exit\" (func (param i32)))",
            "",
            "",
            "",
            "",
        ),
        module("(import \"env\" \"filesystem\" (func))", "", "", "", ""),
        wat::parse_str("(module (func (export \"yami_game_abi\") (result i32) i32.const 2))")
            .unwrap(),
        module("(func $start (loop br 0)) (start $start)", "", "", "", ""),
    ] {
        assert!(Runtime::new(&bytes).is_err());
    }
    let bytes = module("", "", "", "", "");
    let mut vm = Runtime::new(&bytes).unwrap();
    assert_eq!(vm.frame(0, 0).unwrap().commands.len(), 1);
}
#[test]
fn all_guest_entry_points_have_fuel_and_traps_are_errors() {
    for body in ["(loop br 0)", "unreachable"] {
        assert!(Runtime::new(&module("", body, "", "", "")).is_err());
        for (input, update, reset) in [(body, "", ""), ("", body, ""), ("", "", body)] {
            let start = Instant::now();
            let mut vm = Runtime::new(&module("", "", input, update, reset)).unwrap();
            assert!(if reset.is_empty() {
                vm.frame(0, 0).map(|_| ())
            } else {
                vm.reset()
            }
            .is_err());
            assert!(start.elapsed() < Duration::from_secs(2));
        }
    }
}
#[test]
fn memory_table_growth_and_recursion_are_limited() {
    for extra in [
        "(memory 65)",
        "(memory 1) (memory 1)",
        "(table 1025 funcref)",
    ] {
        assert!(
            Runtime::new(&module(extra, "", "", "", "")).is_err(),
            "{extra}"
        );
    }
    for (extra, update) in [
        ("(memory 1)", "i32.const 65 memory.grow drop"),
        (
            "(table 1 funcref)",
            "ref.null func i32.const 1025 table.grow drop",
        ),
        ("(func $recurse call $recurse)", "call $recurse"),
        ("(memory 1)", "i32.const 65536 i32.load drop"),
    ] {
        let mut vm = Runtime::new(&module(extra, "", "", update, "")).unwrap();
        assert!(vm.frame(0, 0).is_err());
    }
}
#[test]
fn drawing_is_bounded_validated_and_transactional() {
    let rect = "(import \"yami_game_v1\" \"rect\" (func $rect (param i32 i32 i32 i32 i32)))";
    for args in [
        "-1 0 1 1 255",
        "0 0 2147483647 1 255",
        "639 0 2 1 255",
        "0 0 0 1 255",
    ] {
        let body = args
            .split_whitespace()
            .map(|s| format!("i32.const {s} "))
            .collect::<String>()
            + "call $rect";
        assert!(Runtime::new(&module(rect, "", "", &body, ""))
            .unwrap()
            .frame(0, 0)
            .is_err());
    }
    let body = "i32.const 0 i32.const 0 i32.const 1 i32.const 1 i32.const 255 call $rect "
        .repeat(MAX_COMMANDS);
    assert!(Runtime::new(&module(rect, "", "", &body, ""))
        .unwrap()
        .frame(0, 0)
        .is_err());
    assert!(Runtime::new(&module("", "i32.const 0 call $clear", "", "", "")).is_err());
    assert!(Runtime::new(&module(
        "",
        "",
        "",
        "i32.const 0 call $clear i32.const 0 call $clear",
        ""
    ))
    .unwrap()
    .frame(0, 0)
    .is_err());
    let text="(import \"yami_game_v1\" \"text\" (func $text (param i32 i32 i32 i32 i32 i32))) (memory (export \"memory\") 1) (data (i32.const 0) \"\\ff\\00\")";
    for (ptr, len, size) in [
        (0, 1, 12),
        (1, 1, 12),
        (65535, 128, 12),
        (-1, 1, 12),
        (0, 129, 12),
        (0, 1, 49),
    ] {
        let body=format!("i32.const {ptr} i32.const {len} i32.const 0 i32.const 0 i32.const {size} i32.const 255 call $text");
        assert!(Runtime::new(&module(text, "", "", &body, ""))
            .unwrap()
            .frame(0, 0)
            .is_err());
    }
    let circle = "(import \"yami_game_v1\" \"circle\" (func $circle (param i32 i32 i32 i32)))";
    assert!(Runtime::new(&module(
        circle,
        "",
        "",
        "i32.const 0 i32.const 0 i32.const 100 i32.const 255 call $circle",
        ""
    ))
    .unwrap()
    .frame(0, 0)
    .is_err());
}
#[test]
fn actual_rust_wasm_game_has_screens_input_collisions_score_lives_and_reset() {
    let (manifest, bytes) = load_local(&sample()).unwrap();
    assert_eq!(manifest.title, "Prism Breakout");
    let mut vm = Runtime::new(&bytes).unwrap();
    let splash = vm.frame(0, 0).unwrap();
    assert!(has_text(&splash, "PRISM BREAKOUT"));
    let ready = vm.frame(0, START).unwrap();
    assert!(has_text(&ready, "SPACE to launch"));
    let moved = vm.frame(LEFT, 0).unwrap();
    fn paddle(f: &Frame) -> i32 {
        f.commands
            .iter()
            .find_map(|c| match c {
                Command::Rect {
                    x, w: 96, h: 10, ..
                } => Some(*x),
                _ => None,
            })
            .unwrap()
    }
    assert!(paddle(&moved) < paddle(&ready));
    let launched = vm.frame(0, START).unwrap();
    assert!(!has_text(&launched, "SPACE to launch"));
    let mut scored = false;
    for _ in 0..400 {
        let f = vm.frame(0, 0).unwrap();
        if !has_text(&f, "SCORE 000") {
            scored = true;
        }
    }
    assert!(
        scored,
        "Ball must destroy bricks and score through actual Wasm"
    );
    // Intentionally move the paddle away and serve each remaining life.
    let mut over = false;
    for i in 0..4000 {
        let f = vm
            .frame(LEFT, if i % 120 == 0 { START } else { 0 })
            .unwrap();
        if has_text(&f, "GAME OVER") {
            assert!(has_text(&f, "LIVES 0"));
            over = true;
            break;
        }
    }
    assert!(over);
    vm.reset().unwrap();
    let reset = vm.frame(0, 0).unwrap();
    assert_eq!(reset.commands, splash.commands);
    let mut independent = Runtime::new(&bytes).unwrap();
    assert_eq!(independent.frame(0, 0).unwrap().commands, splash.commands);
}
#[test]
fn session_pause_resume_stop_and_other_work_remains_responsive() {
    let game = Session::launch(sample(), Arc::new(|| {})).unwrap();
    fn until(game: &Session, check: impl Fn(&Snapshot) -> bool) {
        let start = Instant::now();
        while !check(&game.snapshot()) {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "{:?}",
                game.snapshot()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    until(&game, |s| s.frames >= 3);
    game.key(START, true);
    game.key(START, false);
    until(&game, |s| {
        s.frame
            .as_ref()
            .is_some_and(|f| has_text(f, "SPACE to launch"))
    });
    game.pause();
    until(&game, |s| s.status == Status::Paused);
    let frames = game.snapshot().frames;
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(game.snapshot().frames, frames);
    // Keys pressed while paused must not be replayed on resume.
    game.key(START, true);
    game.resume();
    until(&game, |s| s.frames > frames);
    assert!(has_text(
        game.snapshot().frame.as_ref().unwrap(),
        "SPACE to launch"
    ));
    game.stop();
    until(&game, |s| s.status == Status::Stopped);
}
