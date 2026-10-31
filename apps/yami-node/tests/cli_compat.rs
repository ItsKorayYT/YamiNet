//! Existing yami-node command-line compatibility, exercised through the real
//! executable. Output is redirected to files so the test does not depend on
//! pseudo-terminal or pipe availability.
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-node-cli-compat-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str], base: &Path) -> (bool, String, String) {
    let stdout = base.join("stdout.txt");
    let stderr = base.join("stderr.txt");
    let status = Command::new(env!("CARGO_BIN_EXE_yami-node"))
        .args(args)
        .stdout(Stdio::from(File::create(&stdout).unwrap()))
        .stderr(Stdio::from(File::create(&stderr).unwrap()))
        .status()
        .unwrap();
    (
        status.success(),
        fs::read_to_string(stdout).unwrap_or_default(),
        fs::read_to_string(stderr).unwrap_or_default(),
    )
}

fn valid_config(base: &Path) -> PathBuf {
    let path = base.join("node.conf");
    fs::write(
        &path,
        format!(
            "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\nlisten 127.0.0.1:17444\ndht yes\nadvertise 127.0.0.1:17444\n",
            base.join("keys").display(),
            base.join("state").display(),
            base.join("site").display()
        ),
    )
    .unwrap();
    path
}

#[test]
fn version_help_and_check_config_remain_compatible() {
    let scratch = Scratch::new("main");
    let (ok, stdout, _) = run(&["--version"], &scratch.0);
    assert!(ok);
    assert!(stdout.contains("Yami Node"));
    let (ok, stdout, _) = run(&["--help"], &scratch.0);
    assert!(ok);
    for expected in ["serve-dir", "host-relay", "--config", "check-config"] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
    let config = valid_config(&scratch.0);
    let (ok, stdout, stderr) = run(&["check-config", config.to_str().unwrap()], &scratch.0);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Valid node configuration"));
}

#[test]
fn invalid_configuration_still_fails_closed() {
    let scratch = Scratch::new("invalid");
    let path = scratch.0.join("bad.conf");
    fs::write(&path, "not a yaminet config\n").unwrap();
    let (ok, _, stderr) = run(&["check-config", path.to_str().unwrap()], &scratch.0);
    assert!(!ok);
    assert!(!stderr.is_empty());
}

#[test]
fn host_relay_and_dht_commands_are_still_listed() {
    let scratch = Scratch::new("usage");
    let (ok, stdout, _) = run(&["--help"], &scratch.0);
    assert!(ok);
    for expected in [
        "dht bootstrap",
        "dht lookup",
        "identity new",
        "check-config",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
}
