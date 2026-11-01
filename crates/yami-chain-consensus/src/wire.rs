//! One bounded, versioned codec for channel, WAL and loopback transport messages.
use crate::types::*;
use bytes::Bytes;
use malachite::app::{
    streaming::{StreamContent, StreamId, StreamMessage},
    types::{
        codec::{Codec, HasEncodedLen},
        core as m, sync,
    },
};
use malachite_core::{LivenessMsg, ProposedValue, SignedConsensusMsg};
use malachite_sync as sync_types;
use std::collections::BTreeSet;
use yami_chain::{
    codec::{Reader, Writer},
    Block, Digest, Error, Genesis, Result, MAX_BLOCK_BYTES,
};
use yami_chain_storage::FinalizedRecord;

pub(crate) const MAX_MESSAGE: usize = 1_048_576;
pub(crate) const MAX_SYNC_VALUES: usize = 8;
pub(crate) trait Wire: Sized {
    const TAG: u8;
    fn put(&self, w: &mut Writer) -> Result<()>;
    fn get(r: &mut Reader<'_>) -> Result<Self>;
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct WireCodec;
fn encode<T: Wire>(value: &T) -> Result<Bytes> {
    let mut w = Writer::default();
    w.raw(b"YC-MSG-1");
    w.raw(&Genesis::local().id().0);
    w.u8(T::TAG);
    value.put(&mut w)?;
    if w.0.len() > MAX_MESSAGE {
        return Err(Error::Limit);
    }
    Ok(w.0.into())
}
fn decode<T: Wire>(bytes: Bytes) -> Result<T> {
    let mut r = Reader::new(&bytes, MAX_MESSAGE)?;
    if r.take(8)? != b"YC-MSG-1" || r.array::<32>()? != Genesis::local().id().0 || r.u8()? != T::TAG
    {
        return Err(Error::Encoding);
    }
    let value = T::get(&mut r)?;
    r.end()?;
    Ok(value)
}
macro_rules! codec {
    ($($t:ty),* $(,)?)=>{$(impl Codec<$t> for WireCodec {
        type Error=Error;
        fn encode(&self,value:&$t)->Result<Bytes> {encode(value)}
        fn decode(&self,bytes:Bytes)->Result<$t> {decode(bytes)}
    })*};
}
codec!(
    SignedConsensusMsg<Context>,
    LivenessMsg<Context>,
    Part,
    StreamMessage<Part>,
    m::ValidatorProof<Context>,
    ProposedValue<Context>,
    m::PolkaCertificate<Context>,
    sync::Status<Context>,
    sync::Request<Context>,
    sync::Response<Context>,
    m::CommitCertificate<Context>
);
impl HasEncodedLen<sync::Response<Context>> for WireCodec {
    fn encoded_len(&self, v: &sync::Response<Context>) -> Result<usize> {
        Ok(encode(v)?.len())
    }
}
pub(crate) fn put_round(w: &mut Writer, round: m::Round) {
    match round {
        m::Round::Nil => w.u8(0),
        m::Round::Some(n) => {
            w.u8(1);
            w.u32(n);
        }
    }
}
pub(crate) fn get_round(r: &mut Reader<'_>) -> Result<m::Round> {
    match r.u8()? {
        0 => Ok(m::Round::Nil),
        1 => {
            let n = r.u32()?;
            if n > 1_000_000 {
                return Err(Error::Limit);
            }
            Ok(m::Round::new(n))
        }
        _ => Err(Error::Encoding),
    }
}
fn get_height(r: &mut Reader<'_>) -> Result<Height> {
    let n = r.u64()?;
    if n > 1_000_000 {
        return Err(Error::Limit);
    }
    Ok(Height(n))
}
fn put_value(w: &mut Writer, v: m::NilOrVal<Value>) {
    match v {
        m::NilOrVal::Nil => w.u8(0),
        m::NilOrVal::Val(v) => {
            w.u8(1);
            w.raw(&v.0 .0);
        }
    }
}
fn get_value(r: &mut Reader<'_>) -> Result<m::NilOrVal<Value>> {
    match r.u8()? {
        0 => Ok(m::NilOrVal::Nil),
        1 => Ok(m::NilOrVal::Val(Value(Digest(r.array()?)))),
        _ => Err(Error::Encoding),
    }
}
pub(crate) fn put_vote(w: &mut Writer, v: &Vote) -> Result<()> {
    if v.extension.is_some() {
        return Err(Error::Unsupported);
    }
    w.u64(v.height.0);
    put_round(w, v.round);
    w.u8(match v.step {
        m::VoteType::Prevote => 1,
        m::VoteType::Precommit => 2,
    });
    put_value(w, v.value);
    w.raw(&v.address.0 .0);
    Ok(())
}
fn get_vote(r: &mut Reader<'_>) -> Result<Vote> {
    let height = get_height(r)?;
    let round = get_round(r)?;
    let step = match r.u8()? {
        1 => m::VoteType::Prevote,
        2 => m::VoteType::Precommit,
        _ => return Err(Error::Encoding),
    };
    Ok(Vote {
        height,
        round,
        step,
        value: get_value(r)?,
        address: Address(Digest(r.array()?)),
        extension: None,
    })
}
pub(crate) fn put_proposal(w: &mut Writer, p: &Proposal) {
    w.u64(p.height.0);
    put_round(w, p.round);
    w.raw(&p.value.0 .0);
    put_round(w, p.pol_round);
    w.raw(&p.address.0 .0);
}
fn get_proposal(r: &mut Reader<'_>) -> Result<Proposal> {
    Ok(Proposal {
        height: get_height(r)?,
        round: get_round(r)?,
        value: Value(Digest(r.array()?)),
        pol_round: get_round(r)?,
        address: Address(Digest(r.array()?)),
    })
}
impl Wire for SignedConsensusMsg<Context> {
    const TAG: u8 = 1;
    fn put(&self, w: &mut Writer) -> Result<()> {
        match self {
            Self::Vote(v) => {
                w.u8(1);
                put_vote(w, &v.message)?;
                w.raw(&v.signature);
            }
            Self::Proposal(p) => {
                w.u8(2);
                put_proposal(w, &p.message);
                w.raw(&p.signature);
            }
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        match r.u8()? {
            1 => Ok(Self::Vote(m::SignedMessage::new(get_vote(r)?, r.array()?))),
            2 => Ok(Self::Proposal(m::SignedMessage::new(
                get_proposal(r)?,
                r.array()?,
            ))),
            _ => Err(Error::Encoding),
        }
    }
}
impl Wire for Part {
    const TAG: u8 = 2;
    fn put(&self, w: &mut Writer) -> Result<()> {
        w.u64(self.height.0);
        put_round(w, self.round);
        put_round(w, self.valid_round);
        w.raw(&self.proposer.0 .0);
        w.bytes(&self.block.canonical()?)
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            height: get_height(r)?,
            round: get_round(r)?,
            valid_round: get_round(r)?,
            proposer: Address(Digest(r.array()?)),
            block: Block::decode(r.bytes(MAX_BLOCK_BYTES)?)?,
        })
    }
}
impl Wire for StreamMessage<Part> {
    const TAG: u8 = 3;
    fn put(&self, w: &mut Writer) -> Result<()> {
        if self.stream_id.to_bytes().len() > 32 || self.sequence > 1 {
            return Err(Error::Limit);
        }
        w.bytes(&self.stream_id.to_bytes())?;
        w.u64(self.sequence);
        match &self.content {
            StreamContent::Data(p) => {
                w.u8(1);
                p.put(w)?;
            }
            StreamContent::Fin => w.u8(0),
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let stream_id = StreamId::new(Bytes::copy_from_slice(r.bytes(32)?));
        let sequence = r.u64()?;
        if sequence > 1 {
            return Err(Error::Limit);
        }
        let content = match r.u8()? {
            1 => StreamContent::Data(Part::get(r)?),
            0 => StreamContent::Fin,
            _ => return Err(Error::Encoding),
        };
        Ok(Self::new(stream_id, sequence, content))
    }
}
impl Wire for ProposedValue<Context> {
    const TAG: u8 = 4;
    fn put(&self, w: &mut Writer) -> Result<()> {
        w.u64(self.height.0);
        put_round(w, self.round);
        put_round(w, self.valid_round);
        w.raw(&self.proposer.0 .0);
        w.raw(&self.value.0 .0);
        w.u8(u8::from(self.validity.is_valid()));
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let height = get_height(r)?;
        let round = get_round(r)?;
        let valid_round = get_round(r)?;
        let proposer = Address(Digest(r.array()?));
        let value = Value(Digest(r.array()?));
        let validity = match r.u8()? {
            0 => m::Validity::Invalid,
            1 => m::Validity::Valid,
            _ => return Err(Error::Encoding),
        };
        Ok(Self {
            height,
            round,
            valid_round,
            proposer,
            value,
            validity,
        })
    }
}
fn count(r: &mut Reader<'_>, max: usize) -> Result<usize> {
    let n = r.u32()? as usize;
    if n > max {
        Err(Error::Limit)
    } else {
        Ok(n)
    }
}
impl Wire for m::PolkaCertificate<Context> {
    const TAG: u8 = 5;
    fn put(&self, w: &mut Writer) -> Result<()> {
        if self.polka_signatures.len() > 4 {
            return Err(Error::Limit);
        }
        w.u64(self.height.0);
        put_round(w, self.round);
        w.raw(&self.value_id.0 .0);
        w.u32(self.polka_signatures.len() as u32);
        for s in &self.polka_signatures {
            w.raw(&s.address.0 .0);
            w.raw(&s.signature);
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let height = get_height(r)?;
        let round = get_round(r)?;
        let value_id = Value(Digest(r.array()?));
        let n = count(r, 4)?;
        let mut polka_signatures = Vec::with_capacity(n);
        for _ in 0..n {
            polka_signatures.push(m::PolkaSignature::new(
                Address(Digest(r.array()?)),
                r.array()?,
            ));
        }
        Ok(Self {
            height,
            round,
            value_id,
            polka_signatures,
        })
    }
}
impl Wire for LivenessMsg<Context> {
    const TAG: u8 = 6;
    fn put(&self, w: &mut Writer) -> Result<()> {
        match self {
            Self::Vote(v) => {
                w.u8(1);
                put_vote(w, &v.message)?;
                w.raw(&v.signature);
            }
            Self::PolkaCertificate(p) => {
                w.u8(2);
                p.put(w)?;
            }
            Self::SkipRoundCertificate(c) => {
                if c.round_signatures.len() > 8 {
                    return Err(Error::Limit);
                }
                w.u8(3);
                w.u64(c.height.0);
                put_round(w, c.round);
                w.u8(match c.cert_type {
                    m::RoundCertificateType::Skip => 1,
                    m::RoundCertificateType::Precommit => 2,
                });
                w.u32(c.round_signatures.len() as u32);
                for s in &c.round_signatures {
                    w.u8(match s.vote_type {
                        m::VoteType::Prevote => 1,
                        m::VoteType::Precommit => 2,
                    });
                    put_value(w, s.value_id);
                    w.raw(&s.address.0 .0);
                    w.raw(&s.signature);
                }
            }
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        match r.u8()? {
            1 => Ok(Self::Vote(m::SignedMessage::new(get_vote(r)?, r.array()?))),
            2 => Ok(Self::PolkaCertificate(m::PolkaCertificate::get(r)?)),
            3 => {
                let height = get_height(r)?;
                let round = get_round(r)?;
                let cert_type = match r.u8()? {
                    1 => m::RoundCertificateType::Skip,
                    2 => m::RoundCertificateType::Precommit,
                    _ => return Err(Error::Encoding),
                };
                let n = count(r, 8)?;
                let mut round_signatures = Vec::with_capacity(n);
                for _ in 0..n {
                    let step = match r.u8()? {
                        1 => m::VoteType::Prevote,
                        2 => m::VoteType::Precommit,
                        _ => return Err(Error::Encoding),
                    };
                    round_signatures.push(m::RoundSignature::new(
                        step,
                        get_value(r)?,
                        Address(Digest(r.array()?)),
                        r.array()?,
                    ));
                }
                Ok(Self::SkipRoundCertificate(m::RoundCertificate {
                    height,
                    round,
                    cert_type,
                    round_signatures,
                }))
            }
            _ => Err(Error::Encoding),
        }
    }
}
impl Wire for m::ValidatorProof<Context> {
    const TAG: u8 = 7;
    fn put(&self, w: &mut Writer) -> Result<()> {
        if self.public_key.len() != 32 || self.peer_id.len() > 128 {
            return Err(Error::Limit);
        }
        w.bytes(&self.public_key)?;
        w.bytes(&self.peer_id)?;
        w.raw(&self.signature);
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let public_key = r.bytes(32)?.to_vec();
        if public_key.len() != 32 {
            return Err(Error::Encoding);
        }
        Ok(Self::new(public_key, r.bytes(128)?.to_vec(), r.array()?))
    }
}
impl Wire for m::CommitCertificate<Context> {
    const TAG: u8 = 8;
    fn put(&self, w: &mut Writer) -> Result<()> {
        if self.commit_signatures.len() > 4 {
            return Err(Error::Limit);
        }
        w.u64(self.height.0);
        put_round(w, self.round);
        w.raw(&self.value_id.0 .0);
        let mut signatures = self.commit_signatures.clone();
        signatures.sort_by_key(|s| s.address);
        w.u32(signatures.len() as u32);
        for s in signatures {
            w.raw(&s.address.0 .0);
            w.raw(&s.signature);
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let height = get_height(r)?;
        let round = get_round(r)?;
        let value_id = Value(Digest(r.array()?));
        let n = count(r, 4)?;
        let mut commit_signatures = Vec::with_capacity(n);
        let mut previous = None;
        for _ in 0..n {
            let address = Address(Digest(r.array()?));
            if previous.is_some_and(|p| p >= address) {
                return Err(Error::Encoding);
            }
            previous = Some(address);
            commit_signatures.push(m::CommitSignature::new(address, r.array()?));
        }
        Ok(Self {
            height,
            round,
            value_id,
            commit_signatures,
        })
    }
}
impl Wire for sync::Status<Context> {
    const TAG: u8 = 9;
    fn put(&self, w: &mut Writer) -> Result<()> {
        w.bytes(&self.peer_id.to_bytes())?;
        w.u64(self.tip_height.0);
        w.u64(self.history_min_height.0);
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let peer_id = malachite::app::types::PeerId::from_bytes(r.bytes(128)?)
            .map_err(|_| Error::Encoding)?;
        Ok(Self {
            peer_id,
            tip_height: get_height(r)?,
            history_min_height: get_height(r)?,
        })
    }
}
impl Wire for sync::Request<Context> {
    const TAG: u8 = 10;
    fn put(&self, w: &mut Writer) -> Result<()> {
        let Self::ValueRequest(v) = self;
        w.u64(v.range.start().0);
        w.u64(v.range.end().0);
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let start = get_height(r)?;
        let end = get_height(r)?;
        if start.0 == 0 || end.0 < start.0 || end.0 - start.0 >= MAX_SYNC_VALUES as u64 {
            return Err(Error::Limit);
        }
        Ok(Self::ValueRequest(sync_types::ValueRequest::new(
            start..=end,
        )))
    }
}
impl Wire for sync::Response<Context> {
    const TAG: u8 = 11;
    fn put(&self, w: &mut Writer) -> Result<()> {
        let Self::ValueResponse(v) = self;
        if v.values.len() > MAX_SYNC_VALUES {
            return Err(Error::Limit);
        }
        w.u64(v.start_height.0);
        w.u32(v.values.len() as u32);
        for value in &v.values {
            if value.value_bytes.len() > MAX_BLOCK_BYTES
                || value
                    .certificate
                    .commit_signatures
                    .iter()
                    .any(|s| s.extension.is_some())
            {
                return Err(Error::Limit);
            }
            w.bytes(&value.value_bytes)?;
            value.certificate.trim_vote_extensions().put(w)?;
        }
        Ok(())
    }
    fn get(r: &mut Reader<'_>) -> Result<Self> {
        let start_height = get_height(r)?;
        let n = count(r, MAX_SYNC_VALUES)?;
        let mut values = Vec::with_capacity(n);
        for _ in 0..n {
            let bytes = Bytes::copy_from_slice(r.bytes(MAX_BLOCK_BYTES)?);
            let cert = m::CommitCertificate::get(r)?;
            let certificate = m::ExtendedCommitCertificate::from_commit_certificate_and_extensions(
                cert,
                m::VoteExtensions::default(),
            );
            values.push(sync::RawDecidedValue::new(bytes, certificate));
        }
        Ok(Self::ValueResponse(sync_types::ValueResponse::new(
            start_height,
            values,
        )))
    }
}
pub fn encode_certificate(c: &m::CommitCertificate<Context>) -> Result<Vec<u8>> {
    Ok(encode(c)?.to_vec())
}
pub fn decode_certificate(bytes: &[u8]) -> Result<m::CommitCertificate<Context>> {
    decode(Bytes::copy_from_slice(bytes))
}
pub fn verify_record(record: &FinalizedRecord) -> std::io::Result<()> {
    verify_certificate(
        &record.block,
        &decode_certificate(&record.certificate).map_err(std::io::Error::other)?,
    )
    .map_err(std::io::Error::other)
}
pub(crate) fn verify_certificate(block: &Block, c: &m::CommitCertificate<Context>) -> Result<()> {
    if c.height.0 != block.header.height
        || c.height.0 == 0
        || c.round.is_nil()
        || c.value_id.0 != block.id()?
        || block.header.committee != Genesis::local().committee_hash()
    {
        return Err(Error::Header);
    }
    if c.commit_signatures.len() < 3 || c.commit_signatures.len() > 4 {
        return Err(Error::Signature);
    }
    let mut seen = BTreeSet::new();
    for s in &c.commit_signatures {
        if !seen.insert(s.address) || !Genesis::local().validators.contains(&s.address.0 .0) {
            return Err(Error::Signature);
        }
        let vote = Vote {
            height: c.height,
            round: c.round,
            value: m::NilOrVal::Val(c.value_id),
            step: m::VoteType::Precommit,
            address: s.address,
            extension: None,
        };
        yami_chain::verify(
            &s.address.0 .0,
            &crate::signing::vote_preimage(&vote)?,
            &s.signature,
        )?;
    }
    Ok(())
}
