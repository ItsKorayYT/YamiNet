//! Static and (when a working bash exists) syntax checks for operator scripts.
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn check_script(path: &Path, tokens: &[&str]) {
    let bytes = fs::read(path).unwrap();
    assert!(
        !bytes.contains(&b'\r'),
        "{} must use LF line endings",
        path.display()
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.starts_with("#!/usr/bin/env bash\n"),
        "{} needs a bash shebang",
        path.display()
    );
    for token in tokens {
        assert!(
            text.contains(token),
            "{} is missing safety token {token}",
            path.display()
        );
    }
}

#[test]
fn operator_scripts_have_expected_safety_text_and_line_endings() {
    let root = repo_root();
    let wrapper = root.join("deploy/testnet/yaminet-testnet");
    let rehearsal = root.join("deploy/testnet/scenarios/loopback-rehearsal.sh");
    assert!(wrapper.exists(), "missing {}", wrapper.display());
    assert!(rehearsal.exists(), "missing {}", rehearsal.display());
    check_script(
        &wrapper,
        &["set -euo pipefail", "--purge-data", "--confirm", "firewall"],
    );
    check_script(&rehearsal, &["set -euo pipefail", "firewall"]);
    let wrapper_text = fs::read_to_string(&wrapper).unwrap();
    for token in [
        "install --profile",
        "uninstall",
        "firewall-plan",
        "systemctl enable --now",
        "YAMINET_DRY_RUN",
    ] {
        assert!(wrapper_text.contains(token), "wrapper is missing {token}");
    }
    let rehearsal_text = fs::read_to_string(&rehearsal).unwrap();
    assert!(rehearsal_text.contains("SIMULATION"));
    assert!(rehearsal_text.contains("--route direct"));
}

#[test]
fn documentation_and_templates_are_linked_from_the_kit() {
    let root = repo_root();
    for relative in [
        "docs/public-testnet.md",
        "deploy/testnet/README.md",
        "deploy/testnet/scenarios/relay-fallback.md",
        "deploy/testnet/scenarios/cross-network-checklist.md",
        "deploy/testnet/VALIDATION.md",
        "deploy/testnet/templates/bootstrap.conf.example",
        "deploy/testnet/templates/relay.conf.example",
        "deploy/testnet/templates/host-relay.conf.example",
        "deploy/testnet/templates/client.conf.example",
    ] {
        assert!(root.join(relative).exists(), "missing {relative}");
    }
    let guide = fs::read_to_string(root.join("docs/public-testnet.md")).unwrap();
    for phrase in ["NOT RUN", "independent", "firewall", "node.key", "remove"] {
        assert!(
            guide.to_lowercase().contains(&phrase.to_lowercase()),
            "guide is missing {phrase}"
        );
    }
}

#[test]
fn bash_syntax_check_runs_when_bash_is_usable() {
    let root = repo_root();
    let usable = Command::new("bash")
        .args(["-c", "true"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !usable {
        eprintln!("bash is unavailable or unusable in this environment; syntax check NOT RUN");
        return;
    }
    for relative in [
        "deploy/testnet/yaminet-testnet",
        "deploy/testnet/scenarios/loopback-rehearsal.sh",
    ] {
        let script = root.join(relative).display().to_string().replace('\\', "/");
        let output = Command::new("bash")
            .args(["-n", &script])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            relative,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
