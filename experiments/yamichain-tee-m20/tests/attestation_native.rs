//! Run explicitly with --ignored and YAMI_QVL_LIBRARY/YAMI_QVL_SHA256 from a trusted native build.
//! NO quote is generated or re-signed by these tests. Archived quotes cannot authorize wallets.
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use x25519_dalek::{PublicKey, StaticSecret};
use yamichain_tee_m20::{attestation::*, quorum::*, *};
const RAW: &[u8] = include_bytes!("fixtures/m22/tdx_quote");
const HISTORICAL: u64 = 1_750_377_600; // 2025-06-20T00:00:00Z: archived collateral validity window.
fn collateral() -> Collateral {
    serde_json::from_str(include_str!("fixtures/m22/tdx_quote_collateral.json")).unwrap()
}
fn native() -> IntelQvl {
    let path = PathBuf::from(
        std::env::var("YAMI_QVL_LIBRARY").expect("run native/build_and_test.py first"),
    );
    let pin: Id = hex::decode(std::env::var("YAMI_QVL_SHA256").expect("trusted build hash"))
        .unwrap()
        .try_into()
        .unwrap();
    IntelQvl::load(&path, pin).unwrap()
}
fn policy(r: &TdxReport) -> SoftwarePolicy {
    SoftwarePolicy {
        identity: [1; 32],
        mrtd: r.mrtd.clone(),
        rtmrs: r.rtmrs.clone(),
        config_id: r.config_id.clone(),
        owner: r.owner.clone(),
        owner_config: r.owner_config.clone(),
        td_attributes: r.td_attributes,
        xfam: r.xfam,
        seam_attributes: r.seam_attributes,
        extension: r.extension.clone(),
    }
}
fn setup() -> (WalletGate, SigningKey, Committee, Head) {
    let qvl = native();
    let report = qvl.verify_at(RAW, &collateral(), HISTORICAL).unwrap();
    // These measurements belong to a public fixture, NOT an approved YamiChain image.
    // Fixture policy is used only to test subsequent binding rejection.
    let p = policy(report.report());
    let members = std::array::from_fn(|_| Member {
        signing: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
        transport: PublicKey::from(&StaticSecret::random_from_rng(OsRng)).to_bytes(),
    });
    let c = Committee { chain: 22, members };
    let genesis = Head {
        chain: 22,
        epoch: 0,
        commitment: [7; 32],
    };
    let witness_policy = yamichain_tee_m20::witness::WitnessPolicy {
        native_scope: None,
        committee: c.clone(),
        genesis: genesis.clone(),
        authorities: std::array::from_fn(|_| {
            SigningKey::generate(&mut OsRng).verifying_key().to_bytes()
        }),
        observer: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
    };
    let gate =
        WalletGate::new_witnessed(qvl, witness_policy, std::array::from_fn(|_| p.clone())).unwrap();
    (gate, SigningKey::generate(&mut OsRng), c, genesis)
}
fn transfer(w: &SigningKey) -> Signed {
    Signed::sign(
        Transfer {
            finance: None,

            sender: w.verifying_key().to_bytes(),
            recipient: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
            amount: 10,
            nonce: 0,
            chain: 22,
        },
        w,
    )
}
fn evidence() -> [Option<Evidence>; 4] {
    std::array::from_fn(|_| {
        Some(Evidence {
            quote: RAW.to_vec(),
            collateral: collateral(),
        })
    })
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn genuine_intel_signed_fixture_verifies_at_archived_time_only() {
    let qvl = native();
    let verified = qvl.verify_at(RAW, &collateral(), HISTORICAL).unwrap();
    assert_eq!(verified.verified_at(), HISTORICAL);
    assert_eq!(verified.quote_hash(), hash(RAW));
    verified
        .enforce(&policy(verified.report()), &verified.report().report_data)
        .unwrap();
    println!("REAL Intel QVL: archived authentic TDX fixture, all native stages STATUS_OK at 2025-06-20; no live authorization");
    let err = qvl
        .verify_at(RAW, &collateral(), 1_793_836_800)
        .err()
        .unwrap();
    let failure = err
        .downcast_ref::<QvlFailure>()
        .expect("native expiry rejection");
    assert_eq!(failure.status, 79); // STATUS_SGX_CRL_EXPIRED
    assert!(qvl.verify_at(RAW, &collateral(), 1_735_689_600).is_err()); // not yet valid collateral.
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn signatures_collateral_revocation_data_and_lengths_are_not_host_assertions() {
    let qvl = native();
    for index in [48, 48 + 120, 48 + 136, 48 + 328, 48 + 520, 636] {
        let mut altered = RAW.to_vec();
        altered[index] ^= 1;
        assert!(
            qvl.verify_at(&altered, &collateral(), HISTORICAL).is_err(),
            "altered byte {index}"
        );
    }
    for len in [0, 1, 47, 48, 631, 635, 636, RAW.len() - 1] {
        assert!(qvl
            .verify_at(&RAW[..len], &collateral(), HISTORICAL)
            .is_err());
    }
    let mut trailing = RAW.to_vec();
    trailing.push(0);
    assert!(qvl.verify_at(&trailing, &collateral(), HISTORICAL).is_err());
    for profile in [3u16, 65535] {
        let mut wrong = RAW.to_vec();
        wrong[..2].copy_from_slice(&profile.to_le_bytes());
        assert!(qvl.verify_at(&wrong, &collateral(), HISTORICAL).is_err());
    }
    let mut oversized = RAW.to_vec();
    oversized[632..636].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(qvl
        .verify_at(&oversized, &collateral(), HISTORICAL)
        .is_err());
    for field in 0..5 {
        let mut c = collateral();
        match field {
            0 => {
                c.root_ca_crl.pop();
                c.root_ca_crl.push('0');
            }
            1 => {
                c.pck_crl.pop();
                c.pck_crl.push('0');
            }
            2 => c.tcb_info_signature = String::from_utf8(vec![b'0'; 128]).unwrap(),
            3 => c.qe_identity_signature = String::from_utf8(vec![b'0'; 128]).unwrap(),
            _ => c.tcb_info_issuer_chain = c.tcb_info_issuer_chain.replace('M', "N"),
        };
        assert!(
            qvl.verify_at(RAW, &c, HISTORICAL).is_err(),
            "altered collateral {field}"
        );
    }
    let mut nul = collateral();
    nul.pck_crl.push('\0');
    assert!(qvl.verify_at(RAW, &nul, HISTORICAL).is_err());
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn genuine_outdated_tcb_quote_is_not_relaxed_into_success() {
    let raw = include_bytes!("fixtures/m22/tdx_quote_outdated");
    let c: Collateral = serde_json::from_str(include_str!(
        "fixtures/m22/tdx_quote_outdated_collateral.json"
    ))
    .unwrap();
    let now = 1_771_545_600; // 2026-02-20T00:00:00Z, within archived collateral dates.
    let error = native().verify_at(raw, &c, now).err().unwrap();
    let failure = error
        .downcast_ref::<QvlFailure>()
        .expect("Intel native non-OK status");
    assert_eq!(failure.stage, "quote signature and TCB");
    assert_eq!(failure.status, 49); // STATUS_TCB_NOT_SUPPORTED; never downgrade to success.
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn approved_measurements_debug_policy_and_reportdata_binding_are_mandatory() {
    let qvl = native();
    let v = qvl.verify_at(RAW, &collateral(), HISTORICAL).unwrap();
    let good = policy(v.report());
    for change in 0..10 {
        let mut bad = good.clone();
        match change {
            0 => bad.mrtd[0] ^= 1,
            1..=4 => bad.rtmrs[change - 1][0] ^= 1,
            5 => bad.td_attributes |= 1,
            6 => bad.xfam ^= 1,
            7 => bad.config_id[0] ^= 1,
            8 => bad.owner[0] ^= 1,
            _ => bad.owner_config[0] ^= 1,
        };
        assert!(v.enforce(&bad, &v.report().report_data).is_err());
    }
    let mut wrong = v.report().report_data;
    wrong[0] ^= 1;
    assert!(v.enforce(&good, &wrong).is_err());
    let (_, _, c, genesis) = setup();
    for i in 0..4 {
        let expected = tdx_report_data(&c, i, &genesis, good.identity, [9; 32]).unwrap();
        assert!(v.enforce(&good, &expected).is_err());
    }
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn archived_or_unverified_evidence_never_encrypts_or_submits_and_rounds_are_single_use() {
    let (mut gate, wallet, _, head) = setup();
    let challenge = gate.begin_payment(transfer(&wallet), head.clone()).unwrap();
    let submitted = AtomicBool::new(false);
    assert!(gate
        .finish_and_submit(challenge.id, &evidence(), |_| {
            submitted.store(true, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    assert!(!submitted.load(Ordering::SeqCst));
    assert!(gate.finish(challenge.id, &evidence()).is_err());
    let next = gate.begin_payment(transfer(&wallet), head).unwrap();
    assert_ne!(next.id, challenge.id);
    for i in 0..4 {
        assert_ne!(next.challenges[i], challenge.challenges[i]);
        assert_eq!(next.expected_report_data[i].len(), 64);
    }
    let mut e = evidence();
    e[0].as_mut().unwrap().quote = bytes(&SoftwareEvidence {
        measurement: [1; 32],
        identity: [2; 32],
        key: [3; 32],
        challenge: next.challenges[0],
        issued: 0,
        expires: u64::MAX,
    });
    assert!(gate.finish(next.id, &e).is_err());
    // No round survives wallet reconstruction/restart; archived challenges cannot be reattached.
    let (mut restarted, _, _, _) = setup();
    assert!(restarted.finish(challenge.id, &evidence()).is_err());
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn balance_queries_use_the_same_three_executor_gate() {
    let (mut gate, wallet, _, head) = setup();
    let reply = StaticSecret::random_from_rng(OsRng);
    let mut q = Query {
        account: wallet.verifying_key().to_bytes(),
        reply_key: PublicKey::from(&reply).to_bytes(),
        head,
        challenge: [1; 32],
        signature: vec![],
    };
    q.sign(&wallet);
    let challenge = gate.begin_query(q).unwrap();
    let called = AtomicBool::new(false);
    assert!(gate
        .finish_and_submit(challenge.id, &evidence(), |_| {
            called.store(true, Ordering::SeqCst);
            Ok(())
        })
        .is_err());
    assert!(!called.load(Ordering::SeqCst));
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn wallet_native_library_must_be_pinned_and_hardware_activation_stays_closed() {
    let path = PathBuf::from(std::env::var("YAMI_QVL_LIBRARY").unwrap());
    assert!(IntelQvl::load(&path, [0; 32]).is_err());
    assert!(IntelQvl::load(PathBuf::from("relative.so").as_path(), [0; 32]).is_err());
    assert!(IntelQvl::load(PathBuf::from("/nonexistent/yami-qvl.so").as_path(), [0; 32]).is_err());
    assert!(production_attestation_gate(RAW).is_err());
    assert!(production_attestation_gate(&[]).is_err());
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn actual_wallet_cli_emits_challenges_but_no_payment_for_archived_evidence() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::OpenOptionsExt;
    use std::process::{Command, Stdio};
    let qvl = native();
    let verified = qvl.verify_at(RAW, &collateral(), HISTORICAL).unwrap();
    let p = policy(verified.report());
    let (_, wallet, c, head) = setup();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trusted-wallet-config.json");
    let witness_policy = yamichain_tee_m20::witness::WitnessPolicy {
        native_scope: None,
        committee: c.clone(),
        genesis: head.clone(),
        authorities: std::array::from_fn(|_| {
            SigningKey::generate(&mut OsRng).verifying_key().to_bytes()
        }),
        observer: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
    };
    let config = serde_json::json!({"qvl_library":std::env::var("YAMI_QVL_LIBRARY").unwrap(),"qvl_sha256":hex::decode(std::env::var("YAMI_QVL_SHA256").unwrap()).unwrap(),"committee":c,"genesis":head,"policies":[p.clone(),p.clone(),p.clone(),p],"witness_policy":witness_policy});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    file.write_all(&bytes(&config)).unwrap();
    drop(file);
    let mut child = Command::new(env!("CARGO_BIN_EXE_attested-wallet"))
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    serde_json::to_writer(
        &mut input,
        &serde_json::json!({"Payment":{"transfer":transfer(&wallet),"previous":head}}),
    )
    .unwrap();
    writeln!(input).unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    output.read_line(&mut line).unwrap();
    let challenge: ChallengeSet = serde_json::from_str(&line).unwrap();
    assert!(challenge.expected_report_data.iter().all(|r| r.len() == 64));
    serde_json::to_writer(
        &mut input,
        &serde_json::json!({"round":challenge.id,"evidence":evidence()}),
    )
    .unwrap();
    writeln!(input).unwrap();
    drop(input);
    let mut rest = String::new();
    output.read_to_string(&mut rest).unwrap();
    assert!(
        rest.is_empty(),
        "wallet must not output ciphertext before three genuine verifications"
    );
    let status = child.wait().unwrap();
    assert!(!status.success());
}

#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn m23_missing_malicious_expired_and_mismatched_quotes_do_not_become_eligible() {
    for bad in 0..4 {
        let (mut gate, wallet, c, head) = setup();
        let challenge = gate.begin_payment(transfer(&wallet), head.clone()).unwrap();
        let mut supplied = evidence();
        supplied[3] = None;
        if bad == 0 {
            supplied[0] = None;
        }
        if bad == 1 {
            supplied[0].as_mut().unwrap().quote = bytes(&SoftwareEvidence {
                measurement: [1; 32],
                identity: [2; 32],
                key: c.members[0].transport,
                challenge: challenge.challenges[0],
                issued: 0,
                expires: u64::MAX,
            });
        }
        if bad == 2 {
            supplied[0].as_mut().unwrap().quote[48 + 520] ^= 1;
        }
        // Remaining authentic public quotes have expired collateral AND lack fresh key-bound REPORTDATA.
        let called = AtomicBool::new(false);
        assert!(gate
            .finish_and_submit(challenge.id, &supplied, |_| {
                called.store(true, Ordering::SeqCst);
                Ok(())
            })
            .is_err());
        assert!(!called.load(Ordering::SeqCst));
    }
}
#[test]
#[ignore = "requires explicitly pinned official Intel QVL native build"]
fn m23_attested_identity_binding_includes_the_witness_authority_policy() {
    let verified = native().verify_at(RAW, &collateral(), HISTORICAL).unwrap();
    let p = policy(verified.report());
    let (_, wallet, c, head) = setup();
    let w = yamichain_tee_m20::witness::WitnessPolicy {
        native_scope: None,
        committee: c,
        genesis: head.clone(),
        authorities: std::array::from_fn(|_| {
            SigningKey::generate(&mut OsRng).verifying_key().to_bytes()
        }),
        observer: SigningKey::generate(&mut OsRng).verifying_key().to_bytes(),
    };
    let mut gate =
        WalletGate::new_witnessed(native(), w.clone(), std::array::from_fn(|_| p.clone())).unwrap();
    let challenges = gate.begin_payment(transfer(&wallet), head).unwrap();
    for i in 0..4 {
        let identity = hash(&bytes(&(
            "YAMI-M23-software-and-witness-policy-v1",
            p.identity,
            w.id(),
        )));
        let expected = tdx_report_data(
            &w.committee,
            i,
            &w.genesis,
            identity,
            challenges.challenges[i],
        )
        .unwrap();
        assert_eq!(expected.as_slice(), challenges.expected_report_data[i]);
        let mut wrong = w.clone();
        wrong.authorities[0] = SigningKey::generate(&mut OsRng).verifying_key().to_bytes();
        let wrong_identity = hash(&bytes(&(
            "YAMI-M23-software-and-witness-policy-v1",
            p.identity,
            wrong.id(),
        )));
        assert_ne!(
            expected,
            tdx_report_data(
                &wrong.committee,
                i,
                &wrong.genesis,
                wrong_identity,
                challenges.challenges[i]
            )
            .unwrap()
        );
    }
}
