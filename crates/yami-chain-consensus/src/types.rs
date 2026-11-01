use bytes::Bytes;
use malachite::app::types::core as m;
use yami_chain::{Block, Digest, Genesis, SigningKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Height(pub u64);
impl std::fmt::Display for Height {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl m::Height for Height {
    const ZERO: Self = Self(0);
    const INITIAL: Self = Self(1);
    fn increment_by(&self, n: u64) -> Self {
        Self(self.0.checked_add(n).expect("height overflow"))
    }
    fn decrement_by(&self, n: u64) -> Option<Self> {
        self.0.checked_sub(n).map(Self)
    }
    fn as_u64(&self) -> u64 {
        self.0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Address(pub Digest);
impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl m::Address for Address {}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Value(pub Digest);
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl m::Value for Value {
    type Id = Self;
    fn id(&self) -> Self {
        *self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub height: Height,
    pub round: m::Round,
    pub value: Value,
    pub pol_round: m::Round,
    pub address: Address,
}
impl m::Proposal<Context> for Proposal {
    fn height(&self) -> Height {
        self.height
    }
    fn round(&self) -> m::Round {
        self.round
    }
    fn value(&self) -> &Value {
        &self.value
    }
    fn take_value(self) -> Value {
        self.value
    }
    fn pol_round(&self) -> m::Round {
        self.pol_round
    }
    fn validator_address(&self) -> &Address {
        &self.address
    }
}
#[derive(Clone, Debug)]
pub struct Vote {
    pub height: Height,
    pub round: m::Round,
    pub value: m::NilOrVal<Value>,
    pub step: m::VoteType,
    pub address: Address,
    pub extension: Option<m::SignedExtension<Context>>,
}
impl PartialEq for Vote {
    fn eq(&self, other: &Self) -> bool {
        self.height == other.height
            && self.round == other.round
            && self.value == other.value
            && self.step == other.step
            && self.address == other.address
    }
}
impl Eq for Vote {}
impl m::Vote<Context> for Vote {
    fn height(&self) -> Height {
        self.height
    }
    fn round(&self) -> m::Round {
        self.round
    }
    fn value(&self) -> &m::NilOrVal<Value> {
        &self.value
    }
    fn take_value(self) -> m::NilOrVal<Value> {
        self.value
    }
    fn vote_type(&self) -> m::VoteType {
        self.step
    }
    fn validator_address(&self) -> &Address {
        &self.address
    }
    fn extension(&self) -> Option<&m::SignedExtension<Context>> {
        self.extension.as_ref()
    }
    fn take_extension(&mut self) -> Option<m::SignedExtension<Context>> {
        self.extension.take()
    }
    fn extend(mut self, ext: m::SignedExtension<Context>) -> Self {
        self.extension = Some(ext);
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Part {
    pub height: Height,
    pub round: m::Round,
    pub valid_round: m::Round,
    pub proposer: Address,
    pub block: Block,
}
impl m::ProposalPart<Context> for Part {
    fn is_first(&self) -> bool {
        true
    }
    fn is_last(&self) -> bool {
        true
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validator {
    pub address: Address,
    pub key: [u8; 32],
}
impl m::Validator<Context> for Validator {
    fn address(&self) -> &Address {
        &self.address
    }
    fn public_key(&self) -> &[u8; 32] {
        &self.key
    }
    fn voting_power(&self) -> u64 {
        1
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Committee(pub [Validator; 4]);
impl Committee {
    pub fn local() -> Self {
        let mut keys = Genesis::local().validators;
        keys.sort();
        Self(keys.map(|key| Validator {
            address: Address(Digest(key)),
            key,
        }))
    }
}
impl m::ValidatorSet<Context> for Committee {
    fn count(&self) -> usize {
        4
    }
    fn total_voting_power(&self) -> u64 {
        4
    }
    fn get_by_address(&self, a: &Address) -> Option<&Validator> {
        self.0.iter().find(|v| &v.address == a)
    }
    fn get_by_index(&self, i: usize) -> Option<&Validator> {
        self.0.get(i)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ed25519;
impl m::SigningScheme for Ed25519 {
    type DecodingError = yami_chain::Error;
    type Signature = [u8; 64];
    type PublicKey = [u8; 32];
    type PrivateKey = SigningKey;
    fn decode_signature(bytes: &[u8]) -> Result<Self::Signature, Self::DecodingError> {
        bytes.try_into().map_err(|_| yami_chain::Error::Encoding)
    }
    fn encode_signature(s: &Self::Signature) -> Vec<u8> {
        s.to_vec()
    }
    fn decode_public_key(bytes: &[u8]) -> Result<Self::PublicKey, Self::DecodingError> {
        let key: [u8; 32] = bytes.try_into().map_err(|_| yami_chain::Error::Encoding)?;
        yami_chain::VerifyingKey::from_bytes(&key).map_err(|_| yami_chain::Error::Signature)?;
        Ok(key)
    }
    fn encode_public_key(k: &Self::PublicKey) -> Vec<u8> {
        k.to_vec()
    }
}
#[derive(Clone, Debug)]
pub struct Context;
impl m::Context for Context {
    type Address = Address;
    type Height = Height;
    type ProposalPart = Part;
    type Proposal = Proposal;
    type Validator = Validator;
    type ValidatorSet = Committee;
    type Timeouts = m::LinearTimeouts;
    type Value = Value;
    type Vote = Vote;
    type Extension = Bytes;
    type SigningScheme = Ed25519;
    fn select_proposer<'a>(
        &self,
        set: &'a Committee,
        height: Height,
        round: m::Round,
    ) -> &'a Validator {
        let i = ((height.0.saturating_sub(1) % 4 + u64::from(round.as_u32().unwrap_or(0)) % 4) % 4)
            as usize;
        &set.0[i]
    }
    fn new_proposal(
        &self,
        height: Height,
        round: m::Round,
        value: Value,
        pol_round: m::Round,
        address: Address,
    ) -> Proposal {
        Proposal {
            height,
            round,
            value,
            pol_round,
            address,
        }
    }
    fn new_prevote(
        &self,
        height: Height,
        round: m::Round,
        value: m::NilOrVal<Value>,
        address: Address,
    ) -> Vote {
        Vote {
            height,
            round,
            value,
            step: m::VoteType::Prevote,
            address,
            extension: None,
        }
    }
    fn new_precommit(
        &self,
        height: Height,
        round: m::Round,
        value: m::NilOrVal<Value>,
        address: Address,
    ) -> Vote {
        Vote {
            height,
            round,
            value,
            step: m::VoteType::Precommit,
            address,
            extension: None,
        }
    }
}
