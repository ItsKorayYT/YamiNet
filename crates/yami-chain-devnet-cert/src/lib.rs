//! Versioned insecure-devnet certificate verification shared with validators. No production activation.
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
pub type Id = [u8; 32];
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
fn bytes<T: Serialize>(v: &T) -> Vec<u8> {
    serde_json::to_vec(v).expect("protocol type")
}
fn hash(v: &[u8]) -> Id {
    Sha256::digest(v).into()
}
fn check(v: bool) -> Result<()> {
    if v {
        Ok(())
    } else {
        Err("invalid devnet certificate".into())
    }
}
fn verify(k: Id, m: &[u8], s: &[u8]) -> Result<()> {
    VerifyingKey::from_bytes(&k)?.verify_strict(m, &Signature::from_slice(s)?)?;
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Head {
    pub chain: u64,
    pub epoch: u64,
    pub commitment: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Member {
    pub signing: Id,
    pub transport: Id,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Committee {
    pub chain: u64,
    pub members: [Member; 4],
}
impl Committee {
    pub fn validate(&self) -> Result<()> {
        let mut signing = BTreeSet::new();
        let mut transport = BTreeSet::new();
        for m in &self.members {
            VerifyingKey::from_bytes(&m.signing)?;
            check(
                m.signing != [0; 32]
                    && m.transport != [0; 32]
                    && signing.insert(m.signing)
                    && transport.insert(m.transport),
            )?;
        }
        Ok(())
    }
    pub fn id(&self) -> Id {
        hash(&bytes(&("YAMI-M21-fixed-committee-v1", self)))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Statement {
    pub committee: Id,
    pub previous: Head,
    pub next: Head,
    pub transaction: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_validators: Option<Vec<(Id, Id)>>,
}
impl Statement {
    pub fn message(&self) -> Vec<u8> {
        bytes(&("YAMI-M21-SOFTWARE-approval-v1", self))
    }
    pub fn validate(&self, c: &Committee, previous: &Head) -> Result<()> {
        check(
            self.committee == c.id()
                && self.previous == *previous
                && previous.chain == c.chain
                && self.next.chain == c.chain
                && previous.epoch.checked_add(1) == Some(self.next.epoch),
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub member: Id,
    pub statement: Statement,
    pub signature: Vec<u8>,
}
impl Approval {
    pub fn verify(&self, c: &Committee, previous: &Head) -> Result<()> {
        self.statement.validate(c, previous)?;
        check(c.members.iter().any(|m| m.signing == self.member))?;
        verify(self.member, &self.statement.message(), &self.signature)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Certificate {
    pub statement: Statement,
    pub approvals: Vec<Approval>,
}
impl Certificate {
    /// Filter invalid/equivocating outliers; count distinct identities in each exact statement group.
    pub fn collect(c: &Committee, previous: &Head, votes: Vec<Approval>) -> Result<Self> {
        check(votes.len() <= 16)?;
        let mut groups: Vec<Vec<Approval>> = Vec::new();
        for vote in votes {
            if vote.verify(c, previous).is_ok() {
                if let Some(group) = groups.iter_mut().find(|g| g[0].statement == vote.statement) {
                    if !group.iter().any(|a| a.member == vote.member) {
                        group.push(vote);
                    }
                } else {
                    groups.push(vec![vote]);
                }
            }
        }
        for group in groups {
            if group.len() >= 3 {
                return Self::assemble(c, previous, group);
            }
        }
        Err("no transition quorum".into())
    }
    pub fn assemble(c: &Committee, previous: &Head, approvals: Vec<Approval>) -> Result<Self> {
        // Never silently mix signatures over different results.
        let statement = approvals.first().ok_or("empty quorum")?.statement.clone();
        let cert = Self {
            statement,
            approvals,
        };
        cert.verify(c, previous)?;
        Ok(cert)
    }
    pub fn verify(&self, c: &Committee, previous: &Head) -> Result<()> {
        c.validate()?;
        self.statement.validate(c, previous)?;
        check(self.approvals.len() >= 3 && self.approvals.len() <= 4)?;
        let mut seen = BTreeSet::new();
        for a in &self.approvals {
            check(a.statement == self.statement && seen.insert(a.member))?;
            a.verify(c, previous)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WitnessPolicy {
    pub committee: Committee,
    pub genesis: Head,
    pub authorities: [Id; 4],
    pub observer: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_scope: Option<Id>,
}
impl WitnessPolicy {
    pub fn id(&self) -> Id {
        hash(&bytes(&("YAMI-M23-witness-policy-v1", self)))
    }
    pub fn member(&self,index:usize)->Result<Id> {
        Ok(self.committee.members.get(index).ok_or("member index")?.signing)
    }
    pub fn validate(&self) -> Result<()> {
        self.committee.validate()?;
        check(self.genesis.chain == self.committee.chain && self.genesis.epoch == 0)?;
        let mut seen = std::collections::BTreeSet::new();
        for key in self.authorities {
            VerifyingKey::from_bytes(&key)?;
            check(
                key != [0; 32]
                    && seen.insert(key)
                    && !self.committee.members.iter().any(|m| m.signing == key),
            )?;
        }
        VerifyingKey::from_bytes(&self.observer)?;
        check(!seen.contains(&self.observer))?;
        check(
            !self
                .committee
                .members
                .iter()
                .any(|m| m.signing == self.observer),
        )
    }
    pub fn index(&self, member: Id) -> Result<usize> {
        self.committee
            .members
            .iter()
            .position(|m| m.signing == member)
            .ok_or("foreign member".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Confirmation {
    pub policy: Id,
    pub member: Id,
    pub revision: u64,
    pub statement: Statement,
    pub snapshot_digest: Id,
    pub signature: Vec<u8>,
}
impl Confirmation {
    pub fn message(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-M23-durable-vote-v1",
            self.policy,
            self.member,
            self.revision,
            &self.statement,
            self.snapshot_digest,
        ))
    }
    pub fn verify(&self, p: &WitnessPolicy) -> Result<()> {
        p.validate()?;
        check(self.policy == p.id() && self.revision > 0)?;
        self.statement
            .validate(&p.committee, &self.statement.previous)?;
        verify(
            p.authorities[p.index(self.member)?],
            &self.message(),
            &self.signature,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessedApproval {
    pub approval: Approval,
    pub confirmation: Confirmation,
}
impl WitnessedApproval {
    pub fn verify(&self, p: &WitnessPolicy, head: &Head) -> Result<()> {
        self.approval.verify(&p.committee, head)?;
        self.confirmation.verify(p)?;
        check(
            self.confirmation.member == self.approval.member
                && self.confirmation.statement == self.approval.statement,
        )
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WitnessedCertificate {
    pub certificate: Certificate,
    pub confirmations: Vec<Confirmation>,
}
impl WitnessedCertificate {
    pub fn collect(p: &WitnessPolicy, head: &Head, votes: Vec<WitnessedApproval>) -> Result<Self> {
        let valid: Vec<_> = votes
            .into_iter()
            .filter(|a| a.verify(p, head).is_ok())
            .collect();
        let certificate = Certificate::collect(
            &p.committee,
            head,
            valid.iter().map(|a| a.approval.clone()).collect(),
        )?;
        let confirmations = certificate
            .approvals
            .iter()
            .map(|a| {
                valid
                    .iter()
                    .find(|v| v.approval.member == a.member && v.approval.statement == a.statement)
                    .expect("selected validated vote")
                    .confirmation
                    .clone()
            })
            .collect();
        let result = Self {
            certificate,
            confirmations,
        };
        result.verify(p, head)?;
        Ok(result)
    }
    pub fn verify(&self, p: &WitnessPolicy, head: &Head) -> Result<()> {
        p.validate()?;
        self.certificate.verify(&p.committee, head)?;
        check(self.confirmations.len() == self.certificate.approvals.len())?;
        let mut seen = std::collections::BTreeSet::new();
        for receipt in &self.confirmations {
            receipt.verify(p)?;
            check(
                seen.insert(receipt.member)
                    && receipt.statement == self.certificate.statement
                    && self
                        .certificate
                        .approvals
                        .iter()
                        .any(|a| a.member == receipt.member),
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Ledger {
    pub policy: WitnessPolicy,
    pub head: Head,
    pub next_validators: Option<Vec<(Id, Id)>>,
}
impl Ledger {
    pub fn new(policy: WitnessPolicy) -> Result<Self> {
        policy.validate()?;
        Ok(Self {
            head: policy.genesis.clone(),
            next_validators: None,
            policy,
        })
    }
    pub fn apply(&mut self, payload: &[u8]) -> Result<()> {
        check(payload.len() <= 30000)?;
        let qc: WitnessedCertificate = serde_json::from_slice(payload)?;
        check(bytes(&qc) == payload)?;
        qc.verify(&self.policy, &self.head)?;
        let next = qc.certificate.statement.next_validators;
        check(next.is_some() == self.policy.native_scope.is_some())?;
        check(self.next_validators.is_none() || next.is_some())?;
        if let Some(set) = &next {
            check((4..=8).contains(&set.len()))?;
            let mut keys = BTreeSet::new();
            for (c, t) in set {
                check(
                    !VerifyingKey::from_bytes(c)?.is_weak()
                        && !VerifyingKey::from_bytes(t)?.is_weak()
                        && keys.insert(*c)
                        && keys.insert(*t),
                )?;
            }
            check(set.windows(2).all(|w| w[0].0 < w[1].0))?;
        }
        self.next_validators = next;
        self.head = qc.certificate.statement.next;
        Ok(())
    }
    pub fn commitment(&self) -> Vec<u8> {
        if self.next_validators.is_some() {
            bytes(&(self.policy.id(), &self.head, &self.next_validators))
        } else {
            bytes(&(self.policy.id(), &self.head))
        }
    }
}

pub fn parse_policy(raw: &[u8]) -> Result<WitnessPolicy> {
    Ok(serde_json::from_slice(raw)?)
}
#[cfg(test)]
mod tests;
