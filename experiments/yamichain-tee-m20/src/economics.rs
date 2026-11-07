//! Finite-supply accounting executed inside the existing witnessed private ledger.
//! Opt-in genesis policy only. This does not activate production PoS or change Malachite membership.
use crate::*;
const MAX_VALIDATORS: usize = 128;
const MAX_BONDS: usize = 4096;
const MAX_TICKETS: usize = 4096;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub base_fee: u64,
    pub unit_fee: u64,
    pub max_units: u64,
    pub minimum_stake: u64,
    pub unbonding_blocks: u64,
    pub epoch_blocks: u64,
    pub active_limit: usize,
    pub fee_recipients: Vec<Id>,
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        check(
            self.max_units >= 4
                && self.max_units <= 1024
                && self.minimum_stake > 0
                && self.unbonding_blocks > 0
                && self.epoch_blocks > 0
                && (1..=100).contains(&self.active_limit),
        )?;
        check(!self.fee_recipients.is_empty() && self.fee_recipients.len() <= 100)?;
        let mut seen = std::collections::BTreeSet::new();
        for id in &self.fee_recipients {
            VerifyingKey::from_bytes(id)?;
            check(seen.insert(*id))?;
        }
        self.fee(4)?;
        Ok(())
    }
    pub fn fee(&self, units: u64) -> Result<u64> {
        check(units <= self.max_units)?;
        self.unit_fee
            .checked_mul(units)
            .and_then(|n| n.checked_add(self.base_fee))
            .ok_or("fee overflow".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Action {
    Transfer,
    Register {
        consensus: Id,
        transport: Id,
        consensus_proof: Vec<u8>,
        transport_proof: Vec<u8>,
    },
    Bond {
        validator: Id,
    },
    Unbond {
        validator: Id,
    },
    Withdraw {
        ticket: u64,
    },
    Exit {
        validator: Id,
    },
}
impl Action {
    pub fn units(&self) -> u64 {
        match self {
            Self::Transfer | Self::Withdraw { .. } | Self::Exit { .. } => 1,
            Self::Register { .. } => 4,
            _ => 2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub fee_limit: u64,
    pub action: Action,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Validator {
    pub owner: Id,
    pub consensus: Id,
    pub transport: Id,
    pub exited: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bond {
    pub owner: Id,
    pub validator: Id,
    pub amount: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ticket {
    pub owner: Id,
    pub id: u64,
    pub validator: Id,
    pub amount: u64,
    pub unlock: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Economics {
    pub policy: Policy,
    pub supply: u64,
    pub validators: Vec<Validator>,
    pub bonds: Vec<Bond>,
    pub tickets: Vec<Ticket>,
    pub fees_collected: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Portfolio {
    pub bonded: u64,
    pub unbonding: u64,
    pub withdrawable: u64,
    pub withdrawals: Vec<Ticket>,
}
pub fn registration_message(
    chain: u64,
    owner: Id,
    nonce: u64,
    consensus: Id,
    transport: Id,
) -> Vec<u8> {
    bytes(&(
        "YAMI-DEVNET-validator-registration-v1",
        chain,
        owner,
        nonce,
        consensus,
        transport,
    ))
}
fn credit(s: &mut State, id: Id, n: u64) -> Result<()> {
    let a = s.accounts.entry(id).or_default();
    a.balance = a.balance.checked_add(n).ok_or("balance overflow")?;
    Ok(())
}
fn debit(s: &mut State, id: Id, n: u64) -> Result<()> {
    let a = s.accounts.get_mut(&id).ok_or("unknown account")?;
    a.balance = a.balance.checked_sub(n).ok_or("insufficient funds")?;
    Ok(())
}
impl Economics {
    pub fn new(policy: Policy, accounts: &BTreeMap<Id, Account>) -> Result<Self> {
        policy.validate()?;
        let supply = accounts
            .values()
            .try_fold(0u64, |a, b| a.checked_add(b.balance))
            .ok_or("supply overflow")?;
        Ok(Self {
            policy,
            supply,
            validators: vec![],
            bonds: vec![],
            tickets: vec![],
            fees_collected: 0,
        })
    }
    pub fn invariant(&self, accounts: &BTreeMap<Id, Account>) -> Result<()> {
        self.policy.validate()?;
        check(
            self.validators.len() <= MAX_VALIDATORS
                && self.bonds.len() <= MAX_BONDS
                && self.tickets.len() <= MAX_TICKETS,
        )?;
        let sum = accounts
            .values()
            .map(|a| u128::from(a.balance))
            .chain(self.bonds.iter().map(|b| u128::from(b.amount)))
            .chain(self.tickets.iter().map(|t| u128::from(t.amount)))
            .sum::<u128>();
        check(sum == u128::from(self.supply))?;
        let mut keys = std::collections::BTreeSet::new();
        for v in &self.validators {
            check(
                v.consensus != v.transport && keys.insert(v.consensus) && keys.insert(v.transport),
            )?;
        }
        check(
            self.validators
                .windows(2)
                .all(|v| v[0].consensus < v[1].consensus),
        )?;
        check(
            self.bonds
                .windows(2)
                .all(|b| (b[0].owner, b[0].validator) < (b[1].owner, b[1].validator)),
        )?;
        check(
            self.tickets
                .windows(2)
                .all(|t| (t[0].owner, t[0].id) < (t[1].owner, t[1].id)),
        )?;
        for b in &self.bonds {
            check(b.amount > 0 && self.validators.iter().any(|v| v.consensus == b.validator))?;
        }
        for t in &self.tickets {
            check(t.amount > 0 && self.validators.iter().any(|v| v.consensus == t.validator))?;
        }
        Ok(())
    }
    pub fn portfolio(&self, owner: Id, height: u64) -> Portfolio {
        Portfolio {
            bonded: self
                .bonds
                .iter()
                .filter(|b| b.owner == owner)
                .map(|b| b.amount)
                .sum(),
            unbonding: self
                .tickets
                .iter()
                .filter(|t| t.owner == owner)
                .map(|t| t.amount)
                .sum(),
            withdrawable: self
                .tickets
                .iter()
                .filter(|t| t.owner == owner && t.unlock <= height)
                .map(|t| t.amount)
                .sum(),
            withdrawals: self
                .tickets
                .iter()
                .filter(|t| t.owner == owner)
                .cloned()
                .collect(),
        }
    }
    /// Account-backed eligibility; no peer-advertised weights, seed/beacon or live BFT activation.
    pub fn eligible(&self) -> Result<Vec<(Id, Id, u64)>> {
        let mut result = vec![];
        for v in &self.validators {
            let self_bond = self
                .bonds
                .iter()
                .filter(|b| b.owner == v.owner && b.validator == v.consensus)
                .try_fold(0u64, |n, b| n.checked_add(b.amount))
                .ok_or("stake overflow")?;
            if v.exited || self_bond < self.policy.minimum_stake {
                continue;
            }
            let weight = self
                .bonds
                .iter()
                .filter(|b| b.validator == v.consensus)
                .try_fold(0u64, |n, b| n.checked_add(b.amount))
                .ok_or("stake overflow")?;
            result.push((v.consensus, v.transport, weight));
        }
        Ok(result)
    }
    pub(crate) fn apply(&mut self, s: &mut State, t: &Transfer, intent: &Intent) -> Result<()> {
        self.invariant(&s.accounts)?;
        let fee = self.policy.fee(intent.action.units())?;
        check(intent.fee_limit >= fee)?;
        let withdrawal = matches!(&intent.action, Action::Withdraw { .. });
        if !withdrawal {
            debit(s, t.sender, fee)?;
        }
        match &intent.action {
            Action::Transfer => {
                check(t.amount > 0 && t.sender != t.recipient)?;
                debit(s, t.sender, t.amount)?;
                credit(s, t.recipient, t.amount)?;
            }
            Action::Register {
                consensus,
                transport,
                consensus_proof,
                transport_proof,
            } => {
                check(
                    t.amount == 0
                        && self.validators.len() < MAX_VALIDATORS
                        && consensus != transport
                        && !self.validators.iter().any(|v| {
                            [v.consensus, v.transport]
                                .iter()
                                .any(|k| k == consensus || k == transport)
                        }),
                )?;
                let msg = registration_message(s.chain, t.sender, t.nonce, *consensus, *transport);
                verify(*consensus, &msg, consensus_proof)?;
                verify(*transport, &msg, transport_proof)?;
                self.validators.push(Validator {
                    owner: t.sender,
                    consensus: *consensus,
                    transport: *transport,
                    exited: false,
                });
            }
            Action::Bond { validator } => {
                check(
                    t.amount > 0
                        && self
                            .validators
                            .iter()
                            .any(|v| v.consensus == *validator && !v.exited),
                )?;
                debit(s, t.sender, t.amount)?;
                if let Some(b) = self
                    .bonds
                    .iter_mut()
                    .find(|b| b.owner == t.sender && b.validator == *validator)
                {
                    b.amount = b.amount.checked_add(t.amount).ok_or("bond overflow")?;
                } else {
                    check(self.bonds.len() < MAX_BONDS)?;
                    self.bonds.push(Bond {
                        owner: t.sender,
                        validator: *validator,
                        amount: t.amount,
                    });
                }
            }
            Action::Unbond { validator } => {
                check(t.amount > 0 && self.tickets.len() < MAX_TICKETS)?;
                let b = self
                    .bonds
                    .iter_mut()
                    .find(|b| b.owner == t.sender && b.validator == *validator)
                    .ok_or("no bond")?;
                b.amount = b
                    .amount
                    .checked_sub(t.amount)
                    .ok_or("insufficient bonded stake")?;
                self.tickets.push(Ticket {
                    owner: t.sender,
                    id: t.nonce,
                    validator: *validator,
                    amount: t.amount,
                    unlock: s
                        .epoch
                        .checked_add(1)
                        .and_then(|height| height.checked_add(self.policy.unbonding_blocks))
                        .ok_or("unlock overflow")?,
                });
                self.bonds.retain(|b| b.amount > 0);
            }
            Action::Withdraw { ticket } => {
                check(t.amount == 0)?;
                let i = self
                    .tickets
                    .iter()
                    .position(|v| v.owner == t.sender && v.id == *ticket && v.unlock <= s.epoch)
                    .ok_or("withdrawal unavailable or immature")?;
                let ticket = self.tickets.remove(i);
                credit(s, t.sender, ticket.amount)?;
            }
            Action::Exit { validator } => {
                check(t.amount == 0)?;
                let v = self
                    .validators
                    .iter_mut()
                    .find(|v| v.consensus == *validator && v.owner == t.sender && !v.exited)
                    .ok_or("validator exit unauthorized")?;
                v.exited = true;
            }
        }
        if withdrawal {
            debit(s, t.sender, fee)?;
        }
        let count = self.policy.fee_recipients.len() as u64;
        let offset = s.epoch % count;
        for (i, id) in self.policy.fee_recipients.iter().enumerate() {
            let n = fee / count + u64::from(((i as u64 + count - offset) % count) < fee % count);
            credit(s, *id, n)?;
        }
        self.fees_collected = self
            .fees_collected
            .checked_add(fee)
            .ok_or("fee accounting overflow")?;
        self.validators.sort_by_key(|v| v.consensus);
        self.bonds.sort_by_key(|b| (b.owner, b.validator));
        self.tickets.sort_by_key(|t| (t.owner, t.id));
        self.invariant(&s.accounts)
    }
}
