//! Disposable offline wallets. No production key/address/backup format is proposed.
use std::io::{Read, Write};

use curve25519_dalek::{constants::ED25519_BASEPOINT_POINT, scalar::Scalar as DalekScalar};
use hkdf::Hkdf;
use monero_wallet::{
    address::{MoneroAddress, Network},
    ed25519::{Point, Scalar},
    ViewPair,
};
use rand_core::{OsRng, RngCore};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::{Error, Result};

const BACKUP_MAGIC: &[u8] = b"YC-OFFLINE-WALLET-1";

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Wallet {
    seed: [u8; 32],
    birthday: u64,
}

impl Wallet {
    pub fn generate(birthday: u64) -> Self {
        let mut seed = [0; 32];
        OsRng.fill_bytes(&mut seed);
        Self { seed, birthday }
    }

    fn derive(&self, label: &[u8]) -> Zeroizing<[u8; 64]> {
        let mut key = Zeroizing::new([0; 64]);
        Hkdf::<Sha256>::new(Some(b"yamichain/offline-ringct-wallet/v1"), &self.seed)
            .expand(label, key.as_mut())
            .expect("fixed HKDF output length");
        key
    }

    pub(crate) fn spend_key(&self) -> Zeroizing<Scalar> {
        Zeroizing::new(Scalar::from(DalekScalar::from_bytes_mod_order_wide(
            &self.derive(b"spend"),
        )))
    }

    pub(crate) fn outgoing_key(&self) -> Zeroizing<[u8; 32]> {
        let mut key = Zeroizing::new([0; 32]);
        key.copy_from_slice(&self.derive(b"outgoing")[..32]);
        key
    }

    /// Incoming view authority can scan but cannot sign spends or derive the spend key.
    pub fn view_pair(&self) -> ViewPair {
        let spend = self.spend_key();
        ViewPair::new(
            Point::from((*spend).into() * ED25519_BASEPOINT_POINT),
            Zeroizing::new(Scalar::hash(<[u8; 32]>::from(*spend))),
        )
        .expect("generated prime-order spend key")
    }

    /// Upstream staging-network address, for this experiment only.
    pub fn address(&self) -> MoneroAddress {
        self.view_pair().legacy_address(Network::Stagenet)
    }

    pub fn birthday(&self) -> u64 {
        self.birthday
    }

    /// Standard age X25519 encryption. Keep its identity separately from the archive.
    /// No plaintext key export, filesystem persistence, or secret Debug implementation.
    pub fn backup(&self, recipient: &age::x25519::Recipient) -> Result<Vec<u8>> {
        let mut plaintext = Zeroizing::new(BACKUP_MAGIC.to_vec());
        plaintext.extend(self.birthday.to_be_bytes());
        plaintext.extend_from_slice(&self.seed);
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(recipient as &dyn age::Recipient))
                .map_err(|_| Error::Backup)?;
        let mut ciphertext = Vec::new();
        let mut writer = encryptor
            .wrap_output(&mut ciphertext)
            .map_err(|_| Error::Backup)?;
        writer.write_all(&plaintext).map_err(|_| Error::Backup)?;
        writer.finish().map_err(|_| Error::Backup)?;
        Ok(ciphertext)
    }

    pub fn restore(ciphertext: &[u8], identity: &age::x25519::Identity) -> Result<Self> {
        if ciphertext.len() > 4096 {
            return Err(Error::Limit);
        }
        let decryptor = age::Decryptor::new(ciphertext).map_err(|_| Error::Backup)?;
        let reader = decryptor
            .decrypt(std::iter::once(identity as &dyn age::Identity))
            .map_err(|_| Error::Backup)?;
        let size = BACKUP_MAGIC.len() + 8 + 32;
        let mut plaintext = Zeroizing::new(Vec::new());
        reader
            .take((size + 1) as u64)
            .read_to_end(&mut plaintext)
            .map_err(|_| Error::Backup)?;
        if plaintext.len() != size || !plaintext.starts_with(BACKUP_MAGIC) {
            return Err(Error::Backup);
        }
        let body = &plaintext[BACKUP_MAGIC.len()..];
        Ok(Self {
            birthday: u64::from_be_bytes(body[..8].try_into().map_err(|_| Error::Backup)?),
            seed: body[8..].try_into().map_err(|_| Error::Backup)?,
        })
    }
}
