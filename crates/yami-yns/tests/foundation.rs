use std::{collections::BTreeMap, fs, path::PathBuf};
use yami_identity::{NodeId, NodeIdentity};
use yami_yns::{resolve, Error, History, Name, Record, RecordKind, RecordSource, Store};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let id = NodeIdentity::generate().unwrap();
        Self(std::env::temp_dir().join(format!("yns-{}", id.id())))
    }
    fn open(&self) -> Store {
        Store::open(&self.0).unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn owner() -> NodeIdentity {
    NodeIdentity::generate().unwrap()
}
fn namespace(o: &NodeIdentity) -> String {
    format!("{}.yns", o.id())
}
fn name(o: &NodeIdentity, path: &str) -> Name {
    Name::parse(&format!("{path}.{}", namespace(o))).unwrap()
}
fn mapping(o: &NodeIdentity, ns: &str, path: &str, seq: u64, target: NodeId) -> Record {
    Record::sign(
        o,
        ns,
        path,
        seq,
        100,
        1000,
        RecordKind::Map(target.to_string()),
    )
    .unwrap()
}
#[derive(Default)]
struct Source(BTreeMap<(String, String), Record>);
impl Source {
    fn put(&mut self, r: Record) {
        self.0.insert((r.namespace().into(), r.path().into()), r);
    }
}
impl RecordSource for Source {
    fn lookup(&self, ns: &str, path: &str) -> yami_yns::Result<Option<Record>> {
        Ok(self.0.get(&(ns.into(), path.into())).cloned())
    }
}
#[test]
fn namespace_creation_ownership_and_collisions() {
    let tmp = Temp::new();
    let first = NodeIdentity::create(tmp.0.join("owner")).unwrap();
    assert_eq!(
        first.id(),
        NodeIdentity::load(tmp.0.join("owner")).unwrap().id()
    );
    assert!(NodeIdentity::create(tmp.0.join("owner")).is_err());
    let second = owner();
    let target = *owner().id();
    let mut store = Store::open(tmp.0.join("state")).unwrap();
    for o in [&first, &second] {
        store
            .import(mapping(o, &namespace(o), "blog", 1, target), 200)
            .unwrap();
        assert_eq!(store.resolve(&name(o, "blog"), 200).unwrap(), target);
    }
    assert_ne!(name(&first, "blog"), name(&second, "blog"));
    let forged_owner = mapping(&second, &namespace(&first), "other", 1, target);
    assert!(matches!(
        store.import(forged_owner, 200),
        Err(Error::Signature)
    ));
}
#[test]
fn updates_expiration_rollback_equivocation_and_restart() {
    let tmp = Temp::new();
    let o = owner();
    let target = *owner().id();
    let ns = namespace(&o);
    let old = mapping(&o, &ns, "blog", 1, target);
    let mut store = tmp.open();
    store.import(old.clone(), 200).unwrap();
    store
        .import(mapping(&o, &ns, "blog", 2, target), 200)
        .unwrap();
    assert_eq!(store.resolve(&name(&o, "blog"), 200).unwrap(), target);
    drop(store);
    let mut store = tmp.open();
    assert!(matches!(store.import(old, 200), Err(Error::Rollback)));
    assert!(matches!(
        store.import(mapping(&o, &ns, "blog", 2, *owner().id()), 200),
        Err(Error::Rollback)
    ));
    assert!(matches!(
        store.resolve(&name(&o, "blog"), 1000),
        Err(Error::Expired)
    ));
    assert!(matches!(
        store.resolve(&name(&o, "blog"), 99),
        Err(Error::Expired)
    ));
    // Expiry never erases sequence history.
    assert!(matches!(
        store.import(mapping(&o, &ns, "blog", 1, target), 200),
        Err(Error::Rollback)
    ));
}
#[test]
fn identity_change_requires_exact_explicit_approval() {
    let tmp = Temp::new();
    let o = owner();
    let ns = namespace(&o);
    let n = name(&o, "blog");
    let a = *owner().id();
    let b = *owner().id();
    let mut s = tmp.open();
    s.import(mapping(&o, &ns, "blog", 1, a), 200).unwrap();
    s.resolve(&n, 200).unwrap();
    s.import(mapping(&o, &ns, "blog", 2, b), 200).unwrap();
    assert!(
        matches!(s.resolve(&n,200),Err(Error::IdentityChanged { previous, proposed, .. }) if previous==a && proposed==b)
    );
    drop(s);
    let mut s = tmp.open();
    assert!(matches!(
        s.resolve(&n, 200),
        Err(Error::IdentityChanged { .. })
    ));
    assert!(s.approve_change(&n, b, b, 200).is_err());
    assert!(s.approve_change(&n, a, a, 200).is_err());
    s.approve_change(&n, a, b, 200).unwrap();
    assert_eq!(s.resolve(&n, 200).unwrap(), b);
}
#[test]
fn delegation_is_scoped_and_rotation_keeps_leaf_pins() {
    let tmp = Temp::new();
    let o = owner();
    let delegate = owner();
    let successor = owner();
    let ns = namespace(&o);
    let a = *owner().id();
    let b = *owner().id();
    let delegation = |d: &NodeIdentity, seq| {
        Record::sign(
            &o,
            &ns,
            "team",
            seq,
            100,
            1000,
            RecordKind::Delegate(d.id().to_string()),
        )
        .unwrap()
    };
    let mut s = tmp.open();
    s.import(delegation(&delegate, 1), 200).unwrap();
    s.import(mapping(&delegate, &ns, "www.team", 1, a), 200)
        .unwrap();
    let n = name(&o, "www.team");
    assert_eq!(s.resolve(&n, 200).unwrap(), a);
    assert!(matches!(
        s.import(mapping(&delegate, &ns, "sibling", 1, a), 200),
        Err(Error::Signature)
    ));
    assert!(matches!(
        s.import(mapping(&o, &ns, "www.team", 2, a), 200),
        Err(Error::Signature)
    ));
    s.import(delegation(&successor, 2), 200).unwrap();
    assert!(matches!(s.resolve(&n, 200), Err(Error::Signature)));
    assert!(matches!(
        s.import(mapping(&delegate, &ns, "www.team", 2, a), 200),
        Err(Error::Signature)
    ));
    assert!(matches!(
        s.import(mapping(&successor, &ns, "www.team", 1, b), 200),
        Err(Error::Rollback)
    ));
    s.import(mapping(&successor, &ns, "www.team", 2, b), 200)
        .unwrap();
    assert!(matches!(
        s.resolve(&n, 200),
        Err(Error::IdentityChanged { .. })
    ));
    s.approve_change(&n, a, b, 200).unwrap();
    assert_eq!(s.resolve(&n, 200).unwrap(), b);
}
#[test]
fn tombstones_and_missing_delegations_fail_closed() {
    let o = owner();
    let ns = namespace(&o);
    let mut src = Source::default();
    let mut h = History::default();
    let n = name(&o, "www.team");
    src.put(mapping(&o, &ns, "www.team", 1, *o.id()));
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::NotFound)
    ));
    src.put(Record::sign(&o, &ns, "team", 1, 100, 1000, RecordKind::Revoke).unwrap());
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::NotFound)
    ));
    // The authenticated tombstone was observed even though the leaf failed.
    src.put(
        Record::sign(
            &o,
            &ns,
            "team",
            1,
            100,
            1000,
            RecordKind::Delegate(o.id().to_string()),
        )
        .unwrap(),
    );
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::Rollback)
    ));
}
#[test]
fn local_aliases_are_persistent_and_user_controlled() {
    let tmp = Temp::new();
    let a = *owner().id();
    let b = *owner().id();
    let n = Name::parse("@blog").unwrap();
    let mut s = tmp.open();
    s.set_alias("blog", a, None).unwrap();
    assert!(matches!(
        s.set_alias("blog", b, None),
        Err(Error::IdentityChanged { .. })
    ));
    drop(s);
    let mut s = tmp.open();
    assert_eq!(s.resolve(&n, 200).unwrap(), a);
    assert!(s.set_alias("blog", b, Some(b)).is_err());
    s.set_alias("blog", b, Some(a)).unwrap();
    assert_eq!(s.resolve(&n, 200).unwrap(), b);
    assert!(s.remove_alias("blog", a).is_err());
    s.remove_alias("blog", b).unwrap();
    assert!(matches!(s.resolve(&n, 200), Err(Error::NotFound)));
    assert!(s.set_alias("Blog", a, None).is_err());
}
#[test]
fn official_registry_trust_is_separate_and_empty_by_default() {
    let tmp = Temp::new();
    let registry = owner();
    let ordinary = owner();
    let target = *owner().id();
    let mut s = tmp.open();
    assert!(s.registry_keys().unwrap().is_empty());
    let official = Name::parse("welcome.yami").unwrap();
    assert!(matches!(
        s.resolve(&official, 200),
        Err(Error::UntrustedRegistry)
    ));
    let record = mapping(&registry, "yami", "welcome", 1, target);
    assert!(matches!(
        s.import(record.clone(), 200),
        Err(Error::UntrustedRegistry)
    ));
    s.configure_registry(&[*registry.id()]).unwrap();
    s.import(record, 200).unwrap();
    assert_eq!(s.resolve(&official, 200).unwrap(), target);
    assert!(matches!(
        s.import(mapping(&ordinary, "yami", "welcome", 2, target), 200),
        Err(Error::UntrustedRegistry)
    ));
    s.import(
        mapping(&ordinary, &namespace(&ordinary), "welcome", 1, target),
        200,
    )
    .unwrap();
    s.configure_registry(&[]).unwrap();
    assert_eq!(s.resolve(&name(&ordinary, "welcome"), 200).unwrap(), target);
    assert!(matches!(
        s.resolve(&official, 200),
        Err(Error::UntrustedRegistry)
    ));
}
#[test]
fn malformed_and_forged_records_are_rejected() {
    let o = owner();
    let r = mapping(&o, &namespace(&o), "blog", 1, *o.id());
    assert_eq!(Record::parse(&r.to_bytes()).unwrap(), r);
    let mut wire: serde_json::Value = serde_json::from_slice(&r.to_bytes()).unwrap();
    for (field, value) in [
        ("version", serde_json::json!(2)),
        ("sequence", serde_json::json!(0)),
        ("path", serde_json::json!("Blog")),
        ("signer", serde_json::json!(owner().id().to_string())),
        ("signature", serde_json::json!("00".repeat(64))),
        ("expires", serde_json::json!(2000)),
    ] {
        let mut bad = wire.clone();
        bad[field] = value;
        assert!(
            Record::parse(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{field}"
        );
    }
    wire["unknown"] = serde_json::json!(true);
    assert!(Record::parse(&serde_json::to_vec(&wire).unwrap()).is_err());
    assert!(Record::parse(&vec![b' '; 4097]).is_err());
    assert!(Record::parse(b"{}").is_err());
    let duplicate = String::from_utf8(r.to_bytes()).unwrap().replacen(
        "\"version\":1",
        "\"version\":1,\"version\":1",
        1,
    );
    assert!(Record::parse(duplicate.as_bytes()).is_err());
}
#[test]
fn names_have_one_strict_spelling_and_bounded_depth() {
    let o = owner();
    let ns = namespace(&o);
    for path in [
        "Blog",
        "b_log",
        "xn--blog",
        "-blog",
        "blog-",
        "b%6cog",
        "blоg",
        "blog.",
        ".blog",
        "blog..x",
        "blog/other",
        "blog@other",
        "blog\n",
        "a.b.c.d.e.f.g.h.i",
    ] {
        assert!(Name::parse(&format!("{path}.{ns}")).is_err(), "{path}");
    }
    for input in [
        "blog",
        "BLOG.yami",
        "blog.yami.",
        " blog.yami",
        "blog.yami ",
        "@",
        "@a.b",
        "@yn--é",
        "a.yn1-short.yns",
    ] {
        assert!(Name::parse(input).is_err(), "{input}");
    }
    assert!(Name::parse(&ns).is_ok());
    assert!(Name::parse(&format!("blog.{ns}")).is_ok());
    assert!(Name::parse("@blog-1").is_ok());
    assert!(Name::parse("welcome.yami").is_ok());
}
#[test]
fn source_substitution_and_history_after_failures_are_rejected() {
    let o = owner();
    let ns = namespace(&o);
    let mut src = Source::default();
    let mut h = History::default();
    let target = *o.id();
    let n = name(&o, "blog");
    src.0.insert(
        (ns.clone(), "blog".into()),
        mapping(&o, &ns, "other", 1, target),
    );
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::Signature)
    ));
    src.put(mapping(&o, &ns, "blog", 2, target));
    resolve(&src, &n, &[], &mut h, 200).unwrap();
    src.put(mapping(&o, &ns, "blog", 3, *owner().id()));
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::IdentityChanged { .. })
    ));
    src.put(mapping(&o, &ns, "blog", 2, target));
    assert!(matches!(
        resolve(&src, &n, &[], &mut h, 200),
        Err(Error::Rollback)
    ));
}
#[test]
fn locked_corrupt_or_unwritable_state_never_resets_trust() {
    let tmp = Temp::new();
    let mut s = tmp.open();
    assert!(Store::open(&tmp.0).is_err());
    fs::create_dir(tmp.0.join("yns.tmp")).unwrap();
    assert!(s.set_alias("blog", *owner().id(), None).is_err());
    assert!(s.aliases().is_empty());
    fs::remove_dir(tmp.0.join("yns.tmp")).unwrap();
    s.set_alias("blog", *owner().id(), None).unwrap();
    drop(s);
    fs::write(tmp.0.join("yns.json"), "broken").unwrap();
    assert!(Store::open(&tmp.0).is_err());
}
