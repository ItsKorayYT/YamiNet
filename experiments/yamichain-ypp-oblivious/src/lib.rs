//! Rejected two-server private-write composition, NOT a consensus backend.
//!
//! Dense additive sharing, Pedersen vector commitments, standard Schnorr
//! AND/OR composition, ChaCha20-Poly1305 and hash-authenticated full download.
//! The proof establishes write SHAPE ONLY, not ownership, freshness, conflict
//! authorization, or correct publication of the next storage checkpoint.
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use curve25519_dalek::{
    ristretto::{CompressedRistretto, RistrettoPoint},
    scalar::Scalar,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};

pub type Cell = [Scalar; 2];
pub type WireCell = [[u8; 32]; 2];

fn random_scalar() -> Scalar {
    let mut bytes = [0; 64];
    OsRng.fill_bytes(&mut bytes);
    Scalar::from_bytes_mod_order_wide(&bytes)
}

fn scalar(bytes: &[u8; 32]) -> Result<Scalar, &'static str> {
    Option::from(Scalar::from_canonical_bytes(*bytes)).ok_or("noncanonical scalar")
}

fn point(bytes: &[u8; 32]) -> Result<RistrettoPoint, &'static str> {
    CompressedRistretto(*bytes)
        .decompress()
        .ok_or("invalid point")
}

#[derive(Clone)]
pub struct Parameters {
    g: [RistrettoPoint; 2],
    h: RistrettoPoint,
}

impl Default for Parameters {
    fn default() -> Self {
        // Independent hash-to-group generators; no setup trapdoor.
        Self {
            g: [
                RistrettoPoint::hash_from_bytes::<Sha512>(b"YPP4/Pedersen/g0/v1"),
                RistrettoPoint::hash_from_bytes::<Sha512>(b"YPP4/Pedersen/g1/v1"),
            ],
            h: RistrettoPoint::hash_from_bytes::<Sha512>(b"YPP4/Pedersen/h/v1"),
        }
    }
}

impl Parameters {
    fn commit(&self, value: &Cell, blind: Scalar) -> RistrettoPoint {
        value[0] * self.g[0] + value[1] * self.g[1] + blind * self.h
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShapeProof {
    pub challenges: Vec<[u8; 32]>,
    pub responses: Vec<Vec<[u8; 32]>>,
}

/// ONLY public data; no target, vault key, share vector, or shared randomness.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Packet {
    pub operation: [u8; 32],
    pub commitments: [Vec<[u8; 32]>; 2],
    pub proof: ShapeProof,
}

/// One confidential channel message. A server must not receive both.
#[derive(Clone)]
pub struct PrivateWrite {
    pub delta: Vec<Cell>,
    pub blindings: Vec<Scalar>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WirePrivateWrite {
    pub delta: Vec<WireCell>,
    pub blindings: Vec<[u8; 32]>,
}

impl PrivateWrite {
    pub fn wire(&self) -> WirePrivateWrite {
        WirePrivateWrite {
            delta: self
                .delta
                .iter()
                .map(|c| [c[0].to_bytes(), c[1].to_bytes()])
                .collect(),
            blindings: self.blindings.iter().map(Scalar::to_bytes).collect(),
        }
    }
    pub fn from_wire(wire: &WirePrivateWrite) -> Result<Self, &'static str> {
        Ok(Self {
            delta: wire
                .delta
                .iter()
                .map(|c| Ok([scalar(&c[0])?, scalar(&c[1])?]))
                .collect::<Result<_, &'static str>>()?,
            blindings: wire
                .blindings
                .iter()
                .map(scalar)
                .collect::<Result<_, _>>()?,
        })
    }
}

fn challenge(packet: &Packet, announcements: &[Vec<RistrettoPoint>]) -> Scalar {
    let mut hash = Sha512::new();
    hash.update(b"YPP4/Schnorr-OR-point-write/v1");
    hash.update((packet.commitments[0].len() as u64).to_le_bytes());
    hash.update(packet.operation);
    for server in &packet.commitments {
        for commitment in server {
            hash.update(commitment);
        }
    }
    for branch in announcements {
        for a in branch {
            hash.update(a.compress().as_bytes());
        }
    }
    Scalar::from_hash(hash)
}

/// Prove OR_k AND_{j != k} knowledge of r_j with D_j = r_j H.
/// Thus all but (at most) one aggregate vector commitment open to zero.
/// This is ordinary Schnorr AND/OR composition with Fiat-Shamir, not a new
/// primitive. Its O(N^2) size is deliberately unoptimized.
pub fn prepare(
    parameters: &Parameters,
    n: usize,
    target: usize,
    delta: Cell,
) -> Result<(Packet, [PrivateWrite; 2]), &'static str> {
    if n < 2 || target >= n {
        return Err("invalid dimensions");
    }
    let mut messages = std::array::from_fn(|_| PrivateWrite {
        delta: vec![[Scalar::ZERO; 2]; n],
        blindings: (0..n).map(|_| random_scalar()).collect(),
    });
    let mut commitments: [Vec<[u8; 32]>; 2] = std::array::from_fn(|_| Vec::new());
    for i in 0..n {
        let total = if i == target {
            delta
        } else {
            [Scalar::ZERO; 2]
        };
        for (j, value) in total.into_iter().enumerate() {
            messages[0].delta[i][j] = random_scalar();
            messages[1].delta[i][j] = value - messages[0].delta[i][j];
        }
        for s in 0..2 {
            commitments[s].push(
                parameters
                    .commit(&messages[s].delta[i], messages[s].blindings[i])
                    .compress()
                    .to_bytes(),
            );
        }
    }
    let mut operation = [0; 32];
    OsRng.fill_bytes(&mut operation);
    let mut packet = Packet {
        operation,
        commitments,
        proof: ShapeProof {
            challenges: vec![],
            responses: vec![],
        },
    };
    let aggregate = aggregate(&packet)?;
    let mut c: Vec<_> = (0..n).map(|_| random_scalar()).collect();
    let mut z: Vec<Vec<_>> = (0..n)
        .map(|_| (0..n - 1).map(|_| random_scalar()).collect())
        .collect();
    let mut announcements = vec![];
    for branch in 0..n {
        let a = (0..n)
            .filter(|j| *j != branch)
            .enumerate()
            .map(|(offset, j)| {
                if branch == target {
                    z[branch][offset] * parameters.h
                } else {
                    z[branch][offset] * parameters.h - c[branch] * aggregate[j]
                }
            })
            .collect();
        announcements.push(a);
    }
    c[target] = challenge(&packet, &announcements)
        - c.iter()
            .enumerate()
            .filter(|(i, _)| *i != target)
            .map(|(_, c)| c)
            .sum::<Scalar>();
    for (offset, j) in (0..n).filter(|j| *j != target).enumerate() {
        z[target][offset] += c[target] * (messages[0].blindings[j] + messages[1].blindings[j]);
    }
    packet.proof = ShapeProof {
        challenges: c.iter().map(Scalar::to_bytes).collect(),
        responses: z
            .iter()
            .map(|row| row.iter().map(Scalar::to_bytes).collect())
            .collect(),
    };
    Ok((packet, messages))
}

fn aggregate(packet: &Packet) -> Result<Vec<RistrettoPoint>, &'static str> {
    if packet.commitments[0].len() != packet.commitments[1].len() {
        return Err("commitment dimensions");
    }
    packet.commitments[0]
        .iter()
        .zip(&packet.commitments[1])
        .map(|(a, b)| Ok(point(a)? + point(b)?))
        .collect()
}

pub fn verify(parameters: &Parameters, packet: &Packet) -> Result<(), &'static str> {
    let n = packet.commitments[0].len();
    if n < 2 || packet.proof.challenges.len() != n || packet.proof.responses.len() != n {
        return Err("proof dimensions");
    }
    let aggregate = aggregate(packet)?;
    let c: Vec<_> = packet
        .proof
        .challenges
        .iter()
        .map(scalar)
        .collect::<Result<_, _>>()?;
    let mut announcements = Vec::new();
    for (branch, row) in packet.proof.responses.iter().enumerate() {
        if row.len() != n - 1 {
            return Err("response dimensions");
        }
        let a = (0..n)
            .filter(|j| *j != branch)
            .zip(row)
            .map(|(j, z)| Ok(scalar(z)? * parameters.h - c[branch] * aggregate[j]))
            .collect::<Result<Vec<_>, &'static str>>()?;
        announcements.push(a);
    }
    if c.iter().sum::<Scalar>() != challenge(packet, &announcements) {
        return Err("invalid shape proof");
    }
    Ok(())
}

pub fn verify_share(
    parameters: &Parameters,
    packet: &Packet,
    server: usize,
    message: &PrivateWrite,
) -> Result<(), &'static str> {
    if server > 1
        || message.delta.len() != packet.commitments[server].len()
        || message.blindings.len() != message.delta.len()
    {
        return Err("share dimensions");
    }
    for (i, value) in message.delta.iter().enumerate() {
        if parameters
            .commit(value, message.blindings[i])
            .compress()
            .to_bytes()
            != packet.commitments[server][i]
        {
            return Err("share commitment mismatch");
        }
    }
    Ok(())
}

pub fn delta(new: Cell, old: Cell) -> Cell {
    [new[0] - old[0], new[1] - old[1]]
}

/// The attack gets ONLY the two channel messages. No wallet keys or target.
pub fn reconstruct_support(messages: &[PrivateWrite; 2]) -> Vec<usize> {
    messages[0]
        .delta
        .iter()
        .zip(&messages[1].delta)
        .enumerate()
        .filter(|(_, (a, b))| a[0] + b[0] != Scalar::ZERO || a[1] + b[1] != Scalar::ZERO)
        .map(|(i, _)| i)
        .collect()
}

pub fn private_write_bytes(n: usize) -> usize {
    2 * n * 3 * 32
}
pub fn proof_bytes(n: usize) -> usize {
    32 * n * n
}
pub fn public_packet_bytes(n: usize) -> usize {
    32 + 2 * n * 32 + proof_bytes(n)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlainState {
    pub version: u64,
    pub counter: u64,
}

/// Wallet-only key/index. Servers and public verifier have neither AEAD keys
/// nor plaintext. Secret state is NOT hardened/zeroized in this offline harness.
pub struct Wallet {
    pub index: usize,
    key: [u8; 32],
}

impl Wallet {
    pub fn new(index: usize) -> Self {
        let mut key = [0; 32];
        OsRng.fill_bytes(&mut key);
        Self { index, key }
    }

    pub fn encrypt(&self, value: &PlainState) -> Cell {
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut plain = Vec::new();
        plain.extend(value.version.to_le_bytes());
        plain.extend(value.counter.to_le_bytes());
        let cipher = ChaCha20Poly1305::new((&self.key).into())
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plain,
                    aad: &(self.index as u64).to_le_bytes(),
                },
            )
            .expect("valid AEAD size");
        // 12-byte nonce + 32-byte ciphertext/tag; zero padding to 62 bytes.
        let mut encoded = [0; 62];
        encoded[..12].copy_from_slice(&nonce);
        encoded[12..44].copy_from_slice(&cipher);
        std::array::from_fn(|i| {
            let mut limb = [0; 32];
            limb[..31].copy_from_slice(&encoded[i * 31..(i + 1) * 31]);
            Scalar::from_bytes_mod_order(limb)
        })
    }

    pub fn decrypt(&self, cell: Cell) -> Result<PlainState, &'static str> {
        let mut encoded = Vec::new();
        for limb in cell {
            let bytes = limb.to_bytes();
            if bytes[31] != 0 {
                return Err("invalid ciphertext encoding");
            }
            encoded.extend(&bytes[..31]);
        }
        if encoded[44..].iter().any(|b| *b != 0) {
            return Err("invalid padding");
        }
        let plain = ChaCha20Poly1305::new((&self.key).into())
            .decrypt(
                Nonce::from_slice(&encoded[..12]),
                Payload {
                    msg: &encoded[12..44],
                    aad: &(self.index as u64).to_le_bytes(),
                },
            )
            .map_err(|_| "AEAD authentication failed")?;
        Ok(PlainState {
            version: u64::from_le_bytes(plain[..8].try_into().map_err(|_| "version")?),
            counter: u64::from_le_bytes(plain[8..].try_into().map_err(|_| "counter")?),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub epoch: u64,
    pub cells: Vec<WireCell>,
}

impl Snapshot {
    pub fn hash(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update(b"YPP4/snapshot/v1");
        hash.update(self.epoch.to_le_bytes());
        hash.update((self.cells.len() as u64).to_le_bytes());
        for cell in &self.cells {
            for limb in cell {
                hash.update(limb);
            }
        }
        hash.finalize().into()
    }
}

/// A checkpoint must be obtained from independently finalized history.
/// This harness generates heads using both server states as an honest oracle.
/// The shape proof DOES NOT prove a head's state-transition correctness.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Head {
    pub epoch: u64,
    pub n: usize,
    pub hashes: [[u8; 32]; 2],
}

pub fn recover(
    head: &Head,
    snapshots: &[Snapshot; 2],
    wallet: &Wallet,
) -> Result<PlainState, &'static str> {
    for (s, snapshot) in snapshots.iter().enumerate() {
        if snapshot.epoch != head.epoch
            || snapshot.cells.len() != head.n
            || snapshot.hash() != head.hashes[s]
        {
            return Err("snapshot authentication");
        }
    }
    if wallet.index >= head.n {
        return Err("wallet index");
    }
    let a = snapshots[0].cells[wallet.index];
    let b = snapshots[1].cells[wallet.index];
    wallet.decrypt([
        scalar(&a[0])? + scalar(&b[0])?,
        scalar(&a[1])? + scalar(&b[1])?,
    ])
}

/// Honest-wallet observer: combine downloadable snapshots, compare ciphertexts.
/// Inputs include no target labels, plaintext, AEAD keys or commitment blinds.
pub fn snapshot_support(
    before: &[Snapshot; 2],
    after: &[Snapshot; 2],
) -> Result<Vec<usize>, &'static str> {
    let n = before[0].cells.len();
    if before.iter().chain(after).any(|s| s.cells.len() != n) {
        return Err("snapshot dimensions");
    }
    let mut support = vec![];
    for i in 0..n {
        let mut changed = false;
        for j in 0..2 {
            let old = scalar(&before[0].cells[i][j])? + scalar(&before[1].cells[i][j])?;
            let new = scalar(&after[0].cells[i][j])? + scalar(&after[1].cells[i][j])?;
            changed |= old != new;
        }
        if changed {
            support.push(i);
        }
    }
    Ok(support)
}

/// Offline fixture check requiring a disclosed private message. This is not a
/// zero-knowledge proof that validators can run on the public packet alone.
pub fn verify_snapshot_transition(
    before: &Snapshot,
    after: &Snapshot,
    message: &PrivateWrite,
) -> Result<(), &'static str> {
    if before.epoch.checked_add(1) != Some(after.epoch)
        || before.cells.len() != after.cells.len()
        || before.cells.len() != message.delta.len()
    {
        return Err("transition dimensions/epoch");
    }
    for (i, (a, b)) in before.cells.iter().zip(&after.cells).enumerate() {
        for j in 0..2 {
            if scalar(&a[j])? + message.delta[i][j] != scalar(&b[j])? {
                return Err("snapshot transition mismatch");
            }
        }
    }
    Ok(())
}

/// Honest two-server reference execution, with full rollback on any failed
/// precheck. Real distributed atomic commit / Byzantine availability is absent.
/// Both valid conflicting deltas are deliberately accepted: the counterexample.
#[derive(Clone)]
pub struct StorageLab {
    shares: [Vec<Cell>; 2],
    epoch: u64,
    seen: std::collections::BTreeSet<[u8; 32]>,
}

impl StorageLab {
    pub fn new(ciphertexts: &[Cell]) -> Self {
        let a: Vec<_> = ciphertexts
            .iter()
            .map(|_| [random_scalar(), random_scalar()])
            .collect();
        let b = ciphertexts
            .iter()
            .zip(&a)
            .map(|(cell, share)| delta(*cell, *share))
            .collect();
        Self {
            shares: [a, b],
            epoch: 0,
            seen: Default::default(),
        }
    }

    pub fn snapshots(&self) -> [Snapshot; 2] {
        std::array::from_fn(|s| Snapshot {
            epoch: self.epoch,
            cells: self.shares[s]
                .iter()
                .map(|c| [c[0].to_bytes(), c[1].to_bytes()])
                .collect(),
        })
    }

    pub fn head(&self) -> Head {
        let snapshots = self.snapshots();
        Head {
            epoch: self.epoch,
            n: self.shares[0].len(),
            hashes: snapshots.each_ref().map(Snapshot::hash),
        }
    }

    pub fn apply(
        &mut self,
        parameters: &Parameters,
        packet: &Packet,
        messages: &[PrivateWrite; 2],
    ) -> Result<(), &'static str> {
        if self.seen.contains(&packet.operation) {
            return Err("duplicate operation");
        }
        if packet.commitments[0].len() != self.shares[0].len() {
            return Err("storage dimensions");
        }
        verify(parameters, packet)?;
        for (s, message) in messages.iter().enumerate() {
            verify_share(parameters, packet, s, message)?;
        }
        for (s, message) in messages.iter().enumerate() {
            for (cell, change) in self.shares[s].iter_mut().zip(&message.delta) {
                for j in 0..2 {
                    cell[j] += change[j];
                }
            }
        }
        self.epoch += 1;
        self.seen.insert(packet.operation);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
