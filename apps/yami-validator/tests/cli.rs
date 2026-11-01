use std::process::Command;
#[test]
fn help_and_invalid_commands_preserve_safety_boundary() {
    let binary = env!("CARGO_BIN_EXE_yami-validator");
    let help = Command::new(binary).arg("--help").output().unwrap();
    assert!(help.status.success());
    let text = String::from_utf8(help.stdout).unwrap();
    assert!(text.contains("NOT production-ready; no real money"));
    assert!(text.contains("payments") && text.contains("disabled"));
    for args in [
        vec!["wallet"],
        vec!["run", "--index", "4"],
        vec!["status", "--dir", "x", "--dir", "y"],
        vec!["run", "--public-listen", "0.0.0.0"],
    ] {
        assert!(!Command::new(binary)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    let missing =
        std::env::temp_dir().join(format!("yami-validator-missing-{}", std::process::id()));
    assert!(!Command::new(binary)
        .args(["inspect", "--dir"])
        .arg(&missing)
        .output()
        .unwrap()
        .status
        .success());
    assert!(!missing.exists());
}
