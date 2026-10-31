//! Existing yami-relay command-line compatibility through the real executable.
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yami-relay-cli-compat-{name}-{}",
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
    let status = Command::new(env!("CARGO_BIN_EXE_yami-relay"))
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

#[test]
fn relay_version_help_and_check_config_remain_compatible() {
    let scratch = Scratch::new("main");
    let (ok, stdout, _) = run(&["--version"], &scratch.0);
    assert!(ok);
    assert!(stdout.contains("Yami Relay"));
    let (ok, stdout, _) = run(&["--help"], &scratch.0);
    assert!(ok);
    for expected in ["check-config", "serve", "host", "fetch"] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
    let config = scratch.0.join("relay.conf");
    fs::write(
        &config,
        format!(
            "yaminet-config 1\nrole relay\nnetwork loopback\nidentity {}\nstate {}\nlisten 127.0.0.1:17550\n",
            scratch.0.join("keys").display(),
            scratch.0.join("state").display()
        ),
    )
    .unwrap();
    let (ok, stdout, stderr) = run(&["check-config", config.to_str().unwrap()], &scratch.0);
    assert!(ok, "{stderr}");
    assert!(stdout.contains("Valid relay configuration"));
}

#[test]
fn relay_missing_identity_is_refused_by_startup() {
    let scratch = Scratch::new("identity");
    let config = scratch.0.join("relay.conf");
    fs::write(
        &config,
        format!(
            "yaminet-config 1\nrole relay\nnetwork loopback\nidentity {}\nstate {}\nlisten 127.0.0.1:17550\n",
            scratch.0.join("missing-keys").display(),
            scratch.0.join("state").display()
        ),
    )
    .unwrap();
    let (ok, _, _) = run(&["--config", config.to_str().unwrap()], &scratch.0);
    assert!(!ok);
    assert!(!scratch.0.join("missing-keys/node.key").exists());
}
