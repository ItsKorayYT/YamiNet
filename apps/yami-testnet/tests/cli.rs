//! End-to-end CLI checks for the operator tool itself.
use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::Value;

struct Scratch(PathBuf);
impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("yami-testnet-cli-{name}-{}", std::process::id()));
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

fn run(args: &[String], base: &Path) -> (bool, String, String) {
    let stdout = base.join("stdout.txt");
    let stderr = base.join("stderr.txt");
    let status = Command::new(env!("CARGO_BIN_EXE_yami-testnet"))
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

fn init_profile(scratch: &Scratch) -> PathBuf {
    let profile = scratch.0.join("profile");
    let (ok, stdout, stderr) = run(
        &[
            "init".into(),
            "--profile".into(),
            profile.display().to_string(),
            "--relay".into(),
            "--site".into(),
            "--client".into(),
            "--binary-dir".into(),
            scratch.0.join("bin").display().to_string(),
        ],
        &scratch.0,
    );
    assert!(ok, "{stdout}\n{stderr}");
    assert!(stdout.contains("Initialized testnet profile"));
    profile
}

fn host_url(profile: &Path) -> String {
    let text = fs::read_to_string(profile.join("addresses.env")).unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("HOST_URL='"))
        .and_then(|value| value.strip_suffix('\''))
        .unwrap()
        .to_string()
}

#[test]
fn version_and_help_are_available() {
    let scratch = Scratch::new("help");
    let (ok, stdout, _) = run(&["--version".into()], &scratch.0);
    assert!(ok);
    assert!(stdout.contains("YamiNet Testnet"));
    let (ok, stdout, _) = run(&["--help".into()], &scratch.0);
    assert!(ok);
    for expected in ["preflight", "init", "render-unit", "remove", "NOT VERIFIED"] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
}

#[test]
fn init_check_config_render_and_remove_work_together() {
    let scratch = Scratch::new("workflow");
    let profile = init_profile(&scratch);
    let config = profile.join("bootstrap/bootstrap.conf");
    let (ok, stdout, stderr) = run(
        &[
            "check-config".into(),
            config.display().to_string(),
            "--json".into(),
        ],
        &scratch.0,
    );
    assert!(ok, "{stderr}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["schema"], "yaminet-testnet-report-1");
    assert_eq!(report["result"], "PASS");
    assert_eq!(report["scope"]["independent_machines"], "NOT VERIFIED");

    let unit_output = scratch.0.join("host.service");
    let (ok, _, stderr) = run(
        &[
            "render-unit".into(),
            "--role".into(),
            "host".into(),
            "--config".into(),
            profile.join("host/host.conf").display().to_string(),
            "--output".into(),
            unit_output.display().to_string(),
        ],
        &scratch.0,
    );
    assert!(ok, "{stderr}");
    let unit = fs::read_to_string(unit_output).unwrap();
    assert!(unit.contains("ExecStart="));
    assert!(unit.contains("Restart=on-failure"));
    assert!(unit.contains("SyslogIdentifier=yaminet-testnet-host"));

    let second = run(
        &[
            "init".into(),
            "--profile".into(),
            profile.display().to_string(),
            "--relay".into(),
            "--site".into(),
            "--client".into(),
            "--binary-dir".into(),
            scratch.0.join("bin").display().to_string(),
        ],
        &scratch.0,
    );
    assert!(second.0);
    assert!(second.1.contains("existing files kept"));

    let first_url = host_url(&profile);
    let (ok, _, stderr) = run(
        &[
            "preflight".into(),
            "--config".into(),
            profile.join("client/client.conf").display().to_string(),
            "--url".into(),
            first_url,
            "--route".into(),
            "relay".into(),
            "--timeout".into(),
            "5".into(),
        ],
        &scratch.0,
    );
    let (stdout, _) = {
        let out = scratch.0.join("stdout.txt");
        (fs::read_to_string(out).unwrap_or_default(), ())
    };
    assert!(
        !ok,
        "no server is running, so preflight must fail: {stderr}"
    );
    assert!(stdout.contains("Bootstrap peer reachability"));
    assert!(stdout.contains("NOT RUN"));
    assert!(
        stdout.contains("No plaintext")
            || stdout.contains("no plaintext")
            || stdout.contains("NOT VERIFIED")
    );

    let (ok, stdout, stderr) = run(
        &[
            "remove".into(),
            "--profile".into(),
            profile.display().to_string(),
            "--confirm".into(),
            "--purge-data".into(),
        ],
        &scratch.0,
    );
    assert!(ok, "{stdout}\n{stderr}");
    assert!(!profile.exists());
}

#[test]
fn home_mode_creates_an_outbound_host_without_a_local_bootstrap() {
    let scratch = Scratch::new("home");
    let profile = scratch.0.join("home-profile");
    let bootstrap = yami_identity::NodeIdentity::generate().unwrap();
    let relay = yami_identity::NodeIdentity::generate().unwrap();
    let (ok, stdout, stderr) = run(
        &[
            "init".into(),
            "--profile".into(),
            profile.display().to_string(),
            "--home".into(),
            "--site".into(),
            "--client".into(),
            "--bootstrap-pin".into(),
            format!("{}@8.8.8.8:17444", bootstrap.id()),
            "--relay-pin".into(),
            format!("{}@8.8.8.8:17550", relay.id()),
            "--binary-dir".into(),
            scratch.0.join("bin").display().to_string(),
        ],
        &scratch.0,
    );
    assert!(ok, "{stdout}\n{stderr}");
    assert!(stdout.contains("Mode: home"));
    assert!(profile.join("host/host.conf").exists());
    assert!(profile.join("client/client.conf").exists());
    assert!(!profile.join("bootstrap").exists());
    assert!(!profile.join("relay").exists());
    assert!(profile.join("units/yaminet-testnet-host.service").exists());
    let env = fs::read_to_string(profile.join("addresses.env")).unwrap();
    assert!(env.contains("YAMINET_MODE='home'"));
    assert!(env.contains("HOST_MODE='relay'"));
}

#[test]
fn unsafe_remove_and_missing_identity_fail_without_writes() {
    let scratch = Scratch::new("fail");
    let directory = scratch.0.join("not-a-profile");
    fs::create_dir_all(&directory).unwrap();
    let (ok, _, _) = run(
        &[
            "remove".into(),
            "--profile".into(),
            directory.display().to_string(),
            "--confirm".into(),
            "--purge-data".into(),
        ],
        &scratch.0,
    );
    assert!(!ok);
    assert!(directory.exists());

    let identity = scratch.0.join("missing-identity");
    fs::create_dir_all(&identity).unwrap();
    let config = scratch.0.join("node.conf");
    fs::write(
        &config,
        format!(
            "yaminet-config 1\nrole node\nnetwork loopback\nidentity {}\nstate {}\nsite {}\nlisten 127.0.0.1:17444\n",
            identity.display(),
            scratch.0.join("state").display(),
            scratch.0.join("site").display()
        ),
    )
    .unwrap();
    let (ok, stdout, _) = run(
        &["check-config".into(), config.display().to_string()],
        &scratch.0,
    );
    assert!(!ok);
    assert!(stdout.contains("node.key is missing"));
    assert!(!identity.join("node.key").exists());
}
