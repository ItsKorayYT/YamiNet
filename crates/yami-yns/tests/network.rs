use std::{fs, path::PathBuf};
use yami_dht::Network;
use yami_identity::NodeIdentity;
use yami_yns::{
    network::{self, Provider, Service},
    Bundle, Error, Name, Record, RecordKind, Store,
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("yns02-{}", NodeIdentity::generate().unwrap().id())))
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn bundle(owner: &NodeIdentity, sequence: u64, target: &NodeIdentity) -> Bundle {
    let namespace = format!("{}.yns", owner.id());
    Bundle {
        name: format!("blog.{namespace}"),
        records: vec![Record::sign(
            owner,
            &namespace,
            "blog",
            sequence,
            100,
            1000,
            RecordKind::Map(target.id().to_string()),
        )
        .unwrap()],
    }
}
#[test]
fn signed_chains_survive_mirroring_but_forgery_and_expiration_do_not() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let target = NodeIdentity::generate().unwrap();
    let mut s = Store::open(&tmp.0).unwrap();
    let b = bundle(&owner, 1, &target);
    let n = Name::parse(&b.name).unwrap();
    s.import_bundle(&b, 200).unwrap();
    assert_eq!(s.export_bundle(&n, 200).unwrap().unwrap(), b);
    assert!(s.export_bundle(&n, 1000).unwrap().is_none());
    assert!(matches!(b.verify(&[], 99), Err(Error::Expired)));
    assert!(matches!(b.verify(&[], 1000), Err(Error::Expired)));
    let mut bad: serde_json::Value =
        serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
    bad["records"][0]["action"]["value"] = serde_json::json!(owner.id().to_string());
    let forged: Bundle = serde_json::from_value(bad).unwrap();
    assert!(matches!(
        s.import_bundle(&forged, 200),
        Err(Error::Signature)
    ));
    let mut bad = b.clone();
    bad.name = format!("other.{}.yns", owner.id());
    assert!(bad.verify(&[], 200).is_err());
    let mut bad = b.clone();
    bad.records = vec![b.records[0].clone(); 9];
    assert!(bad.verify(&[], 200).is_err());
}
#[test]
fn refresh_selects_newest_preserves_pins_and_persists_conflicts() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let first = NodeIdentity::generate().unwrap();
    let second = NodeIdentity::generate().unwrap();
    let old = bundle(&owner, 1, &first);
    let current = bundle(&owner, 2, &second);
    let n = Name::parse(&old.name).unwrap();
    let mut s = Store::open(&tmp.0).unwrap();
    s.import_bundle(&old, 200).unwrap();
    s.resolve(&n, 200).unwrap();
    s.refresh(&n, &[current.clone(), old.clone()], 200).unwrap();
    assert!(matches!(
        s.resolve(&n, 200),
        Err(Error::IdentityChanged { .. })
    ));
    assert!(matches!(s.import_bundle(&old, 200), Err(Error::Rollback)));
    s.approve_change(&n, *first.id(), *second.id(), 200)
        .unwrap();
    let conflict = bundle(&owner, 2, &first);
    assert!(matches!(
        s.refresh(&n, &[current, conflict], 200),
        Err(Error::Rollback)
    ));
    drop(s);
    let mut s = Store::open(&tmp.0).unwrap();
    assert!(matches!(s.resolve(&n, 200), Err(Error::Rollback)));
    assert!(s.export_bundle(&n, 200).unwrap().is_none());
    s.refresh(&n, &[bundle(&owner, 3, &second)], 200).unwrap();
    assert_eq!(s.resolve(&n, 200).unwrap(), *second.id());
}
#[test]
fn delegation_revocation_prefix_invalidates_cached_descendants() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let delegate = NodeIdentity::generate().unwrap();
    let target = NodeIdentity::generate().unwrap();
    let namespace = format!("{}.yns", owner.id());
    let n = Name::parse(&format!("www.team.{namespace}")).unwrap();
    let root = Record::sign(
        &owner,
        &namespace,
        "team",
        1,
        100,
        1000,
        RecordKind::Delegate(delegate.id().to_string()),
    )
    .unwrap();
    let leaf = Record::sign(
        &delegate,
        &namespace,
        "www.team",
        1,
        100,
        1000,
        RecordKind::Map(target.id().to_string()),
    )
    .unwrap();
    let mut s = Store::open(&tmp.0).unwrap();
    s.import_bundle(
        &Bundle {
            name: n.to_string(),
            records: vec![root, leaf],
        },
        200,
    )
    .unwrap();
    s.resolve(&n, 200).unwrap();
    let prefix = Bundle {
        name: n.to_string(),
        records: vec![
            Record::sign(&owner, &namespace, "team", 2, 100, 1000, RecordKind::Revoke).unwrap(),
        ],
    };
    s.refresh(&n, std::slice::from_ref(&prefix), 200).unwrap();
    assert!(matches!(s.resolve(&n, 200), Err(Error::NotFound)));
    assert_eq!(s.export_bundle(&n, 200).unwrap().unwrap(), prefix);
}
#[test]
fn provider_pins_migrate_old_profiles_and_do_not_cross_network_policy() {
    let tmp = Temp::new();
    let mut s = Store::open(&tmp.0).unwrap();
    let owner = NodeIdentity::generate().unwrap();
    s.set_alias("blog", *owner.id(), None).unwrap();
    drop(s);
    let mut old: serde_json::Value =
        serde_json::from_slice(&fs::read(tmp.0.join("yns.json")).unwrap()).unwrap();
    for field in ["providers", "refreshed"] {
        old.as_object_mut().unwrap().remove(field);
    }
    old["history"].as_object_mut().unwrap().remove("conflicts");
    fs::write(tmp.0.join("yns.json"), serde_json::to_vec(&old).unwrap()).unwrap();
    let mut s = Store::open(&tmp.0).unwrap();
    assert_eq!(
        s.resolve(&Name::parse("@blog").unwrap(), 200).unwrap(),
        *owner.id()
    );
    let local =
        Provider::parse(&format!("{}@127.0.0.1:7401", owner.id()), Network::Loopback).unwrap();
    assert!(local.contact(Network::Public).is_err());
    assert!(Provider::parse(&format!("{}@10.0.0.1:7401", owner.id()), Network::Public).is_err());
    assert!(s
        .configure_providers(vec![local.clone(); 9], Network::Loopback)
        .is_err());
    s.configure_providers(vec![local.clone()], Network::Loopback)
        .unwrap();
    drop(s);
    assert_eq!(Store::open(&tmp.0).unwrap().providers(), &[local]);
}
fn envelope(message: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":1,"message":message})).unwrap()
}
#[test]
fn mirror_protocol_rejects_malformed_requests_and_enforces_publication_quota() {
    let tmp = Temp::new();
    let s = Service::open(&tmp.0, Network::Loopback, vec![]).unwrap();
    let owner = NodeIdentity::generate().unwrap();
    let b = bundle(&owner, 1, &owner);
    for bytes in [
        vec![b' '; ynp::MAX_NAMING_BYTES + 1],
        b"{}".to_vec(),
        b"{\"version\":2,\"message\":{\"op\":\"stored\"}}".to_vec(),
        envelope(serde_json::json!({"op":"query","name":"@private"})),
        envelope(serde_json::json!({"op":"stored"})),
        envelope(serde_json::json!({"op":"query","name":b.name,"extra":true})),
    ] {
        assert!(s.handle(&bytes, 200).is_err());
    }
    let request = envelope(serde_json::json!({"op":"publish","bundle":b}));
    for _ in 0..network::MAX_PUBLICATIONS_PER_MINUTE {
        let answer: serde_json::Value =
            serde_json::from_slice(&s.handle(&request, 200).unwrap()).unwrap();
        assert_eq!(answer["message"]["op"], "stored");
    }
    let rejected: serde_json::Value =
        serde_json::from_slice(&s.handle(&request, 200).unwrap()).unwrap();
    assert_eq!(rejected["message"]["op"], "rejected");
    let answer: serde_json::Value = serde_json::from_slice(
        &s.handle(
            &envelope(serde_json::json!({"op":"query","name":b.name})),
            200,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(answer["message"]["answer"]["bundle"]["name"], b.name);
}

#[test]
fn older_conflicts_cannot_reduce_a_persisted_conflict_barrier() {
    let tmp = Temp::new();
    let owner = NodeIdentity::generate().unwrap();
    let a = NodeIdentity::generate().unwrap();
    let b = NodeIdentity::generate().unwrap();
    let n = Name::parse(&bundle(&owner, 1, &a).name).unwrap();
    let mut s = Store::open(&tmp.0).unwrap();
    s.import_bundle(&bundle(&owner, 1, &a), 200).unwrap();
    assert!(s
        .refresh(&n, &[bundle(&owner, 3, &a), bundle(&owner, 3, &b)], 200)
        .is_err());
    assert!(s
        .refresh(&n, &[bundle(&owner, 2, &a), bundle(&owner, 2, &b)], 200)
        .is_err());
    assert!(matches!(
        s.import_bundle(&bundle(&owner, 3, &a), 200),
        Err(Error::Rollback)
    ));
    s.import_bundle(&bundle(&owner, 4, &a), 200).unwrap();
    assert_eq!(s.resolve(&n, 200).unwrap(), *a.id());
}

#[test]
fn oversized_persistent_state_and_record_counts_fail_before_validation_work() {
    let tmp = Temp::new();
    fs::create_dir_all(&tmp.0).unwrap();
    fs::write(tmp.0.join("yns.json"), vec![b' '; 4 * 1024 * 1024 + 1]).unwrap();
    assert!(Store::open(&tmp.0).is_err());
    let owner = NodeIdentity::generate().unwrap();
    let record = bundle(&owner, 1, &owner).records.remove(0);
    let records: std::collections::BTreeMap<_, _> =
        (0..4097).map(|i| (i.to_string(), record.clone())).collect();
    let bytes=serde_json::to_vec(&serde_json::json!({"version":1,"aliases":{},"registry":[],"records":records,"history":{"high_water":{},"bindings":{}}})).unwrap();
    assert!(bytes.len() < 4 * 1024 * 1024);
    fs::write(tmp.0.join("yns.json"), bytes).unwrap();
    assert!(Store::open(&tmp.0).is_err());
}
