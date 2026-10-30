//! Real configured relay executable and read-only local status. Loopback only.
#[cfg(unix)]
#[test]
fn configured_relay_requires_existing_identity_and_reports_shutdown_status() {
    use std::{
        fs,
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    use yami_config::status::RelayStatus;
    use yami_identity::NodeIdentity;
    let base = std::env::temp_dir().join(format!("yami-relay-config-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    struct Guard(std::path::PathBuf, Option<Child>);
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Some(child) = &mut self.1 {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let mut guard = Guard(base.clone(), None);
    let selected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = selected.local_addr().unwrap();
    drop(selected);
    let config = base.join("relay.conf");
    let state = base.join("state");
    fs::write(
        &config,
        format!(
            "yaminet-config 1\nrole relay\nidentity {}\nstate {}\nlisten {endpoint}\n",
            base.join("keys").display(),
            state.display()
        ),
    )
    .unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .arg("--config")
        .arg(&config)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(!base.join("keys/node.key").exists());
    let identity = NodeIdentity::create(base.join("keys")).unwrap();
    assert!(Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .arg("check-config")
        .arg(&config)
        .status()
        .unwrap()
        .success());
    guard.1 = Some(
        Command::new(env!("CARGO_BIN_EXE_yami-relay"))
            .arg("--config")
            .arg(&config)
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let until = Instant::now() + Duration::from_secs(5);
    while RelayStatus::read(&state).is_err() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(25));
    }
    assert_eq!(RelayStatus::read(&state).unwrap().id, *identity.id());
    let output = Command::new(env!("CARGO_BIN_EXE_yami-relay"))
        .args(["status", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("recently running"));
    let child = guard.1.as_mut().unwrap();
    assert!(Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap()
        .success());
    assert!(child.wait().unwrap().success());
    assert!(!RelayStatus::read(&state).unwrap().running);
    assert_eq!(RelayStatus::read(&state).unwrap().connections, 0);
}
