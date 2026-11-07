use super::*;
use ed25519_dalek::{Signer, SigningKey};

fn fixture() -> (WitnessPolicy, WitnessedCertificate) {
    let keys: Vec<_> = (1..=4).map(|n| SigningKey::from_bytes(&[n; 32])).collect();
    let witnesses: Vec<_> = (11..=14)
        .map(|n| SigningKey::from_bytes(&[n; 32]))
        .collect();
    let policy = WitnessPolicy {
        committee: Committee {
            chain: 24,
            members: std::array::from_fn(|i| Member {
                signing: keys[i].verifying_key().to_bytes(),
                transport: [i as u8 + 21; 32],
            }),
        },
        genesis: Head {
            chain: 24,
            epoch: 0,
            commitment: [42; 32],
        },
        authorities: std::array::from_fn(|i| witnesses[i].verifying_key().to_bytes()),
        observer: SigningKey::from_bytes(&[31; 32]).verifying_key().to_bytes(),
    };
    let statement = Statement {
        committee: policy.committee.id(),
        previous: policy.genesis.clone(),
        next: Head {
            chain: 24,
            epoch: 1,
            commitment: [43; 32],
        },
        transaction: [44; 32],
    };
    let approvals = (0..3)
        .map(|i| Approval {
            member: keys[i].verifying_key().to_bytes(),
            statement: statement.clone(),
            signature: keys[i].sign(&statement.message()).to_bytes().to_vec(),
        })
        .collect();
    let confirmations = (0..3)
        .map(|i| {
            let mut c = Confirmation {
                policy: policy.id(),
                member: keys[i].verifying_key().to_bytes(),
                revision: 1,
                statement: statement.clone(),
                snapshot_digest: [45; 32],
                signature: vec![],
            };
            c.signature = witnesses[i].sign(&c.message()).to_bytes().to_vec();
            c
        })
        .collect();
    (
        policy,
        WitnessedCertificate {
            certificate: Certificate {
                statement,
                approvals,
            },
            confirmations,
        },
    )
}
#[test]
fn genuine_software_signatures_advance_once_and_reject_replay() {
    let (policy, qc) = fixture();
    let mut ledger = Ledger::new(policy).unwrap();
    ledger.apply(&bytes(&qc)).unwrap();
    let accepted = ledger.commitment();
    assert!(ledger.apply(&bytes(&qc)).is_err());
    assert_eq!(ledger.commitment(), accepted);
}
#[test]
fn hostile_certificate_fields_cannot_change_state() {
    let (policy, qc) = fixture();
    let mut attacks = Vec::new();
    let mut c = qc.clone();
    c.certificate.approvals[0].signature[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.confirmations[0].signature[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.approvals.pop();
    c.confirmations.pop();
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.approvals[1] = c.certificate.approvals[0].clone();
    attacks.push(c);
    let mut c = qc.clone();
    c.confirmations[1] = c.confirmations[0].clone();
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.statement.next.commitment[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.statement.previous.commitment[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.statement.next.epoch += 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.statement.committee[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.certificate.statement.next.chain += 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.confirmations[0].revision = 0;
    attacks.push(c);
    let mut c = qc.clone();
    c.confirmations[0].snapshot_digest[0] ^= 1;
    attacks.push(c);
    let mut c = qc.clone();
    c.confirmations[0].policy[0] ^= 1;
    attacks.push(c);
    for c in attacks {
        let mut ledger = Ledger::new(policy.clone()).unwrap();
        let before = ledger.commitment();
        assert!(ledger.apply(&bytes(&c)).is_err());
        assert_eq!(before, ledger.commitment());
    }
    let mut ledger = Ledger::new(policy.clone()).unwrap();
    assert!(ledger.apply(b"unverifiable-hash").is_err());
    assert!(ledger.apply(&vec![0; 30001]).is_err());
    let mut foreign = policy;
    foreign.committee.members[3].transport = [55; 32];
    assert!(Ledger::new(foreign).unwrap().apply(&bytes(&qc)).is_err());
}

#[test]
fn alternate_certificate_serializations_cannot_poison_native_queue_identity() {
    let (policy, qc) = fixture();
    let mut ledger = Ledger::new(policy).unwrap();
    let before = ledger.commitment();
    assert!(ledger
        .apply(&serde_json::to_vec_pretty(&qc).unwrap())
        .is_err());
    assert_eq!(before, ledger.commitment());
    ledger.apply(&bytes(&qc)).unwrap();
}
