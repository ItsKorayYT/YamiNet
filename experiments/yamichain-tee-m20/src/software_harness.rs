//! Test/development process protocol. Not a network API or production daemon.
use crate::{quorum::*, *};
#[derive(Clone, Serialize, Deserialize)]
pub struct SoftwareConfig {
    pub snapshot: PathBuf,
    pub trusted_anchor: PathBuf,
    pub committee: Committee,
    pub index: usize,
    pub signing_secret: Id,
    pub transport_secret: Id,
    pub storage_secret: Id,
    pub root_secret: Id,
    /// Explicit attacker fixture: this process exposes its own signing authority.
    pub compromised: bool,
}
impl SoftwareConfig {
    pub fn keys(&self) -> ReplicaKeys {
        ReplicaKeys {
            signing: SigningKey::from_bytes(&self.signing_secret),
            transport: StaticSecret::from(self.transport_secret),
            storage: self.storage_secret,
            root_secret: self.root_secret,
        }
    }
}
#[derive(Serialize, Deserialize)]
pub enum Command {
    Prepare(Box<Payment>),
    Commit(Certificate),
    Query(Envelope),
    Head,
    ExportRecovery(usize),
    Recover(Envelope),
    CompromisedSign(Statement),
    CompromisedBalance(Query, Account),
}
#[derive(Serialize, Deserialize)]
pub enum Reply {
    Approval(Approval),
    Committed,
    Balance(Envelope),
    Recovery(Envelope),
    Head(Head),
    Rejected,
}
