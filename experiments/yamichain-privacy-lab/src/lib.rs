//! Isolated real RingCT experiment. This crate cannot authorize YAMI or chain effects.
//! Synthetic funding and in-memory membership are trusted fixtures, not consensus state.
mod fixture;
pub mod wallet;
pub use fixture::{funded_fixture, with_decoys};

use std::collections::BTreeSet;

use curve25519_dalek::{
    edwards::EdwardsPoint,
    traits::{Identity, IsIdentity},
};
use monero_wallet::{
    block::{Block, BlockHeader},
    ed25519::{Commitment, CompressedPoint, Scalar},
    extra::Extra,
    interface::{FeeRate, ScannableBlock},
    ringct::{bulletproofs::Bulletproof, RctPrunable, RctType},
    send::{Change, SignableTransaction},
    transaction::{Input, Timelock, Transaction, TransactionPrefix},
    OutputWithDecoys, Scanner, ViewPair, WalletOutput,
};
use rand_core::OsRng;
use sha2::{Digest, Sha256};
use wallet::Wallet;

pub const MAX_PACKET_BYTES: usize = 16_384;
const MAGIC: &[u8] = b"YAMI-OFFLINE-RINGCT-PAYMENT-1";
const CONTEXT_BYTES: usize = MAGIC.len() + 32 + 32 + 8 + 8 + 8;
pub type PublicOutput = [CompressedPoint; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Encoding,
    Limit,
    Binding,
    Anchor,
    Expired,
    Spent,
    Proof,
    Conservation,
    Authorization,
    Backup,
}
pub type Result<T> = std::result::Result<T, Error>;

fn digest(domain: &[u8], data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update((domain.len() as u64).to_be_bytes());
    h.update(domain);
    h.update((data.len() as u64).to_be_bytes());
    h.update(data);
    h.finalize().into()
}

/// Fixed-domain, single-action experiment context. Not a YamiChain wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub intent: [u8; 32],
    pub anchor: [u8; 32],
    pub expiry: u64,
    pub fee_epoch: u64,
    pub fee: u64,
}

impl Context {
    fn encode(&self) -> Vec<u8> {
        let mut bytes = MAGIC.to_vec();
        bytes.extend(self.intent);
        bytes.extend(self.anchor);
        bytes.extend(self.expiry.to_be_bytes());
        bytes.extend(self.fee_epoch.to_be_bytes());
        bytes.extend(self.fee.to_be_bytes());
        bytes
    }

    fn binding(&self) -> [u8; 32] {
        digest(b"yamichain/offline-ringct/context/v1", &self.encode())
    }
}

#[derive(Clone)]
pub struct Packet {
    pub context: Context,
    pub transaction: Transaction,
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let tx = self.transaction.serialize();
        let mut bytes = self.context.encode();
        bytes.extend((tx.len() as u32).to_be_bytes());
        bytes.extend(tx);
        bytes
    }

    /// Bounds before parsing; exact re-encoding rejects noncanonical/trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(Error::Limit);
        }
        if bytes.len() < CONTEXT_BYTES + 4 || !bytes.starts_with(MAGIC) {
            return Err(Error::Encoding);
        }
        let mut body = &bytes[MAGIC.len()..];
        fn take<const N: usize>(body: &mut &[u8]) -> Result<[u8; N]> {
            let value = body.get(..N).ok_or(Error::Encoding)?;
            let value = value.try_into().map_err(|_| Error::Encoding)?;
            *body = &body[N..];
            Ok(value)
        }
        let context = Context {
            intent: take(&mut body)?,
            anchor: take(&mut body)?,
            expiry: u64::from_be_bytes(take(&mut body)?),
            fee_epoch: u64::from_be_bytes(take(&mut body)?),
            fee: u64::from_be_bytes(take(&mut body)?),
        };
        let length = u32::from_be_bytes(take(&mut body)?) as usize;
        if length != body.len() {
            return Err(Error::Encoding);
        }
        let transaction = Transaction::read(&mut body).map_err(|_| Error::Encoding)?;
        let packet = Self {
            context,
            transaction,
        };
        if !body.is_empty() || packet.encode() != bytes {
            return Err(Error::Encoding);
        }
        Ok(packet)
    }
}

/// Public fixture records and exact spent images. No plaintext balances or openings.
#[derive(Clone, Default)]
pub struct LabState {
    pub(crate) outputs: Vec<PublicOutput>,
    spent: BTreeSet<[u8; 32]>,
}

impl LabState {
    pub fn anchor(&self) -> [u8; 32] {
        let mut bytes = (self.outputs.len() as u64).to_be_bytes().to_vec();
        for output in &self.outputs {
            for point in output {
                bytes.extend(point.to_bytes());
            }
        }
        digest(b"yamichain/offline-ringct/fixture-membership/v1", &bytes)
    }

    pub fn spent_count(&self) -> usize {
        self.spent.len()
    }

    /// Verify ownership, trusted-set membership, ranges, conservation, and full binding.
    /// Upstream range verification uses OsRng: NOT a deterministic consensus verifier.
    pub fn verify(&self, packet: &Packet, height: u64) -> Result<()> {
        if packet.encode().len() > MAX_PACKET_BYTES {
            return Err(Error::Limit);
        }
        let tx = &packet.transaction;
        let prefix = tx.prefix();
        if !(1..=2).contains(&prefix.inputs.len())
            || !(2..=16).contains(&prefix.outputs.len())
            || prefix.additional_timelock != Timelock::None
            || prefix.extra.len() > 1024
        {
            return Err(Error::Limit);
        }
        let mut images = BTreeSet::new();
        for input in &prefix.inputs {
            let Input::ToKey {
                amount: None,
                key_offsets,
                key_image,
            } = input
            else {
                return Err(Error::Encoding);
            };
            if key_offsets.len() != 16 {
                return Err(Error::Limit);
            }
            if self.spent.contains(&key_image.to_bytes()) || !images.insert(key_image.to_bytes()) {
                return Err(Error::Spent);
            }
        }
        if packet.context.anchor != self.anchor() {
            return Err(Error::Anchor);
        }
        if height > packet.context.expiry {
            return Err(Error::Expired);
        }
        let extra = Extra::read(&mut prefix.extra.as_slice()).map_err(|_| Error::Encoding)?;
        if extra.arbitrary_data() != vec![packet.context.binding().to_vec()] {
            return Err(Error::Binding);
        }
        let Transaction::V2 {
            proofs: Some(proofs),
            ..
        } = tx
        else {
            return Err(Error::Proof);
        };
        if proofs.rct_type() != RctType::ClsagBulletproofPlus
            || proofs.base.fee != packet.context.fee
            || !proofs.base.pseudo_outs.is_empty()
            || proofs.base.commitments.len() != prefix.outputs.len()
            || proofs.base.encrypted_amounts.len() != prefix.outputs.len()
            || prefix.outputs.iter().any(|o| o.amount.is_some())
        {
            return Err(Error::Binding);
        }
        for output in &prefix.outputs {
            let key = output.key.decompress().ok_or(Error::Encoding)?.into();
            if key.is_identity() || !key.is_torsion_free() || output.view_tag.is_none() {
                return Err(Error::Encoding);
            }
        }
        let RctPrunable::Clsag {
            clsags,
            pseudo_outs,
            bulletproof,
        } = &proofs.prunable
        else {
            return Err(Error::Proof);
        };
        if clsags.len() != prefix.inputs.len()
            || pseudo_outs.len() != prefix.inputs.len()
            || !matches!(bulletproof, Bulletproof::Plus(_))
        {
            return Err(Error::Proof);
        }
        // No secret openings are used here: sum(Cin) = sum(Cout) + fee*H.
        let sum = |points: &[CompressedPoint]| -> Result<EdwardsPoint> {
            points
                .iter()
                .try_fold(EdwardsPoint::identity(), |total, p| {
                    let p = p.decompress().ok_or(Error::Proof)?.into();
                    if !p.is_torsion_free() {
                        return Err(Error::Proof);
                    }
                    Ok(total + p)
                })
        };
        let fee = Commitment::new(Scalar::ZERO, proofs.base.fee)
            .commit()
            .into();
        if sum(pseudo_outs)? != sum(&proofs.base.commitments)? + fee {
            return Err(Error::Conservation);
        }
        if !bulletproof.verify(&mut OsRng, &proofs.base.commitments) {
            return Err(Error::Proof);
        }
        let message = tx.signature_hash().ok_or(Error::Proof)?;
        for ((input, clsag), pseudo_out) in prefix.inputs.iter().zip(clsags).zip(pseudo_outs) {
            let Input::ToKey {
                key_offsets,
                key_image,
                ..
            } = input
            else {
                return Err(Error::Encoding);
            };
            let mut position = 0_u64;
            let mut ring = Vec::new();
            for (i, offset) in key_offsets.iter().enumerate() {
                if i != 0 && *offset == 0 {
                    return Err(Error::Encoding);
                }
                position = position.checked_add(*offset).ok_or(Error::Encoding)?;
                ring.push(
                    *self
                        .outputs
                        .get(usize::try_from(position).map_err(|_| Error::Limit)?)
                        .ok_or(Error::Anchor)?,
                );
            }
            clsag
                .verify(ring, key_image, pseudo_out, &message)
                .map_err(|_| Error::Proof)?;
        }
        Ok(())
    }

    /// Mutate only after the entire packet verifies, including every input.
    pub fn apply(&mut self, packet: &Packet, height: u64) -> Result<u64> {
        self.verify(packet, height)?;
        let first = self.outputs.len() as u64;
        let Transaction::V2 {
            prefix,
            proofs: Some(proofs),
        } = &packet.transaction
        else {
            return Err(Error::Proof);
        };
        for input in &prefix.inputs {
            if let Input::ToKey { key_image, .. } = input {
                self.spent.insert(key_image.to_bytes());
            }
        }
        self.outputs.extend(
            prefix
                .outputs
                .iter()
                .zip(&proofs.base.commitments)
                .map(|(output, commitment)| [output.key, *commitment]),
        );
        Ok(first)
    }
}

/// Upstream creates stealth outputs, encrypts amounts, proves ranges and signs CLSAGs.
pub fn create_payment(
    sender: &Wallet,
    inputs: Vec<OutputWithDecoys>,
    recipient: &monero_wallet::address::MoneroAddress,
    amount: u64,
    mut context: Context,
) -> Result<Packet> {
    let build = |context: &Context| {
        SignableTransaction::new(
            RctType::ClsagBulletproofPlus,
            sender.outgoing_key(),
            inputs.clone(),
            vec![(*recipient, amount)],
            Change::new(sender.view_pair(), None),
            vec![context.binding().to_vec()],
            FeeRate::new(1, 1).expect("fixed lab fee rate"),
        )
        .map_err(|_| Error::Conservation)
    };
    // The digest has a fixed length, so binding the actual fee doesn't change weight.
    context.fee = build(&context)?.necessary_fee();
    let unsigned = build(&context)?;
    if unsigned.necessary_fee() != context.fee {
        return Err(Error::Binding);
    }
    let transaction = unsigned
        .sign(&mut OsRng, &sender.spend_key())
        .map_err(|_| Error::Authorization)?;
    Ok(Packet {
        context,
        transaction,
    })
}

/// Scan already-verified, locally held data. This is NOT authenticated network sync.
/// A fresh view pair and complete packet history reproduce the same received notes.
pub fn discover(pair: ViewPair, packet: &Packet, first_output: u64) -> Result<Vec<WalletOutput>> {
    let miner = Transaction::V2 {
        prefix: TransactionPrefix {
            additional_timelock: Timelock::None,
            inputs: vec![Input::Gen(0)],
            outputs: vec![],
            extra: vec![],
        },
        proofs: None,
    };
    let block = Block::new(
        BlockHeader {
            hardfork_version: 16,
            hardfork_signal: 16,
            timestamp: 0,
            previous: [0; 32],
            nonce: 0,
        },
        miner,
        vec![packet.transaction.hash()],
    )
    .ok_or(Error::Encoding)?;
    Scanner::new(pair)
        .scan(ScannableBlock {
            block,
            transactions: vec![packet.transaction.clone().into()],
            output_index_for_first_ringct_output: Some(first_output),
        })
        .map(|outputs| outputs.not_additionally_locked())
        .map_err(|_| Error::Encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_binding_known_answer() {
        // Independent Python hashlib vector: u64 BE lengths, domain, context.
        let context = Context {
            intent: [7; 32],
            anchor: [9; 32],
            expiry: 10,
            fee_epoch: 3,
            fee: 1600,
        };
        assert_eq!(
            context.binding(),
            [
                91, 245, 186, 27, 129, 206, 145, 53, 152, 10, 95, 222, 149, 78, 129, 126, 167, 32,
                216, 31, 17, 162, 222, 91, 24, 168, 219, 9, 123, 144, 106, 195,
            ]
        );
    }
}
