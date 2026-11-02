//! Synthetic initial outputs: no mining, issuance API, RPC, or on-chain funds.
use curve25519_dalek::constants::ED25519_BASEPOINT_POINT;
use monero_wallet::{
    ed25519::{Commitment, Point, Scalar},
    ringct::clsag::Decoys,
    OutputWithDecoys, WalletOutput,
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

use crate::{wallet::Wallet, Error, LabState, Result};

fn encode_input(
    state: &LabState,
    position: usize,
    key_offset: Scalar,
    commitment: &Commitment,
) -> Result<OutputWithDecoys> {
    // Adapt the exact pinned upstream OutputWithDecoys storage format. This is
    // secret fixture/wallet data, never a public packet or financial verifier input.
    let mut positions: Vec<usize> = (0..16).collect();
    if position >= 16 {
        positions[15] = position;
    }
    let ring = positions
        .iter()
        .map(|i| {
            let output = state.outputs.get(*i).ok_or(Error::Anchor)?;
            Ok([
                output[0].decompress().ok_or(Error::Encoding)?,
                output[1].decompress().ok_or(Error::Encoding)?,
            ])
        })
        .collect::<Result<Vec<_>>>()?;
    let signer = positions
        .iter()
        .position(|i| *i == position)
        .ok_or(Error::Anchor)?;
    let mut prior = 0;
    let offsets = positions
        .into_iter()
        .map(|i| {
            let offset = (i - prior) as u64;
            prior = i;
            offset
        })
        .collect();
    let decoys = Decoys::new(offsets, signer as u8, ring).ok_or(Error::Encoding)?;
    let mut bytes = Zeroizing::new(Vec::new());
    bytes.extend(state.outputs[position][0].to_bytes());
    key_offset.write(&mut *bytes).map_err(|_| Error::Encoding)?;
    commitment.write(&mut *bytes).map_err(|_| Error::Encoding)?;
    decoys.write(&mut *bytes).map_err(|_| Error::Encoding)?;
    OutputWithDecoys::read(&mut bytes.as_slice()).map_err(|_| Error::Encoding)
}

pub fn funded_fixture(
    wallet: &Wallet,
    amounts: &[u64],
) -> Result<(LabState, Vec<OutputWithDecoys>)> {
    if !(1..=2).contains(&amounts.len()) {
        return Err(Error::Limit);
    }
    let mut state = LabState::default();
    let mut openings = Zeroizing::new(Vec::new());
    for i in 0..16 {
        let offset = Zeroizing::new(Scalar::random(&mut OsRng));
        let secret = Zeroizing::new(if i < amounts.len() {
            (*wallet.spend_key()).into() + (*offset).into()
        } else {
            Scalar::random(&mut OsRng).into()
        });
        let key = Point::from(*secret * ED25519_BASEPOINT_POINT);
        let commitment = Commitment::new(
            Scalar::random(&mut OsRng),
            amounts.get(i).copied().unwrap_or_else(|| OsRng.next_u64()),
        );
        state
            .outputs
            .push([key.compress(), commitment.commit().compress()]);
        if i < amounts.len() {
            openings.push((*offset, commitment));
        }
    }
    let inputs = openings
        .iter()
        .enumerate()
        .map(|(i, (offset, commitment))| encode_input(&state, i, *offset, commitment))
        .collect::<Result<Vec<_>>>()?;
    Ok((state, inputs))
}

pub fn with_decoys(state: &LabState, output: &WalletOutput) -> Result<OutputWithDecoys> {
    let position = usize::try_from(output.index_on_blockchain()).map_err(|_| Error::Limit)?;
    let expected = [
        output.key().compress(),
        output.commitment().commit().compress(),
    ];
    if state.outputs.get(position) != Some(&expected) {
        return Err(Error::Anchor);
    }
    encode_input(state, position, output.key_offset(), output.commitment())
}
