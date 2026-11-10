//! Local wallet key protection. Not an enclave or a substitute for trusted wallet OS security.
use crate::*;
use chacha20poly1305::aead::Payload;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vault {
    pub version: u16,
    pub address: String,
    pub salt: [u8; 32],
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}
const N: u64 = 131072;
fn derive(password: &str, salt: &[u8; 32]) -> Result<[u8; 32]> {
    check(!password.is_empty() && password.len() <= 1024)?;
    let mut key = [0; 32];
    openssl::pkcs5::scrypt(
        password.as_bytes(),
        salt,
        N,
        8,
        1,
        256 * 1024 * 1024,
        &mut key,
    )?;
    Ok(key)
}
impl Vault {
    fn aad(&self) -> Vec<u8> {
        bytes(&(
            "YAMI-wallet-keystore-v1",
            self.version,
            &self.address,
            N,
            8u64,
            1u64,
        ))
    }
    pub fn seal(secret: Id, password: &str) -> Result<Self> {
        check(password.len() >= 12 && password.len() <= 1024)?;
        let mut salt = [0; 32];
        OsRng.fill_bytes(&mut salt);
        let mut nonce = [0; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut v = Self {
            version: 1,
            address: hex::encode(SigningKey::from_bytes(&secret).verifying_key().to_bytes()),
            salt,
            nonce,
            ciphertext: vec![],
        };
        let mut key = derive(password, &salt)?;
        v.ciphertext = ChaCha20Poly1305::new((&key).into())
            .encrypt(
                (&v.nonce).into(),
                Payload {
                    msg: &secret,
                    aad: &v.aad(),
                },
            )
            .map_err(|_| "wallet encryption failed")?;
        key.fill(0);
        Ok(v)
    }
    pub fn unlock(&self, password: &str) -> Result<Id> {
        check(
            self.version == 1
                && self.ciphertext.len() == 48
                && self.address.len() == 64
                && self
                    .address
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        )?;
        let mut key = derive(password, &self.salt)?;
        let attempt = ChaCha20Poly1305::new((&key).into()).decrypt(
            (&self.nonce).into(),
            Payload {
                msg: &self.ciphertext,
                aad: &self.aad(),
            },
        );
        key.fill(0);
        let raw = attempt.map_err(|_| "wallet password or authenticated keystore invalid")?;
        let secret: Id = raw.try_into().map_err(|_| "wallet seed length")?;
        check(
            hex::encode(SigningKey::from_bytes(&secret).verifying_key().to_bytes()) == self.address,
        )?;
        Ok(secret)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_wallet_backup_restores_same_key_and_rejects_tampering() {
        let secret = SigningKey::generate(&mut OsRng).to_bytes();
        let password = "test only long password";
        let v = Vault::seal(secret, password).unwrap();
        assert_eq!(v.unlock(password).unwrap(), secret);
        let backup = bytes(&v);
        let restored: Vault = serde_json::from_slice(&backup).unwrap();
        assert_eq!(restored.unlock(password).unwrap(), secret);
        assert!(v.unlock("wrong password").is_err());
        let mut c = v.clone();
        c.ciphertext[0] ^= 1;
        assert!(c.unlock(password).is_err());
        let mut c = v.clone();
        c.address = "ab".repeat(32);
        assert!(c.unlock(password).is_err());
        let mut c = v.clone();
        c.version = 2;
        assert!(c.unlock(password).is_err());
        assert!(Vault::seal(secret, "short").is_err());
    }
}

/// Wallet-local history encryption uses a separate key and a pinned network
/// context. It grants no authority and cannot establish transaction finality.
pub fn seal_history(secret: Id, network: Id, entries: &serde_json::Value) -> Result<Envelope> {
    let raw = serde_json::to_vec(entries)?;
    check(
        entries.is_array()
            && entries.as_array().is_some_and(|a| a.len() <= 1024)
            && raw.len() <= 2 * 1024 * 1024,
    )?;
    let key = history_key(secret, network)?;
    encrypt(
        PublicKey::from(&key).to_bytes(),
        &raw,
        &history_context(secret, network),
    )
}
pub fn open_history(secret: Id, network: Id, envelope: &Envelope) -> Result<serde_json::Value> {
    check(envelope.ciphertext.len() <= 2 * 1024 * 1024 + 16)?;
    let raw = decrypt(
        &history_key(secret, network)?,
        envelope,
        &history_context(secret, network),
    )?;
    let entries: serde_json::Value = serde_json::from_slice(&raw)?;
    check(entries.as_array().is_some_and(|a| a.len() <= 1024))?;
    Ok(entries)
}
fn history_key(secret: Id, network: Id) -> Result<StaticSecret> {
    let mut key = [0; 32];
    Hkdf::<Sha256>::new(Some(&network), &secret)
        .expand(b"YAMI-wallet-local-history-key-v1", &mut key)
        .map_err(|_| "history key derivation")?;
    Ok(StaticSecret::from(key))
}
fn history_context(secret: Id, network: Id) -> Vec<u8> {
    bytes(&(
        "YAMI-wallet-local-history-v1",
        network,
        SigningKey::from_bytes(&secret).verifying_key().to_bytes(),
    ))
}
#[cfg(test)]
mod history_tests {
    use super::*;
    #[test]
    fn wallet_history_is_private_network_bound_and_tamper_evident() {
        let entries = serde_json::json!([{"recipient":"private recipient", "amount":12345}]);
        let sealed = seal_history([1; 32], [2; 32], &entries).unwrap();
        assert_eq!(open_history([1; 32], [2; 32], &sealed).unwrap(), entries);
        assert!(!String::from_utf8(bytes(&sealed))
            .unwrap()
            .contains("recipient"));
        assert!(open_history([3; 32], [2; 32], &sealed).is_err());
        assert!(open_history([1; 32], [3; 32], &sealed).is_err());
        let mut changed = sealed;
        changed.ciphertext[0] ^= 1;
        assert!(open_history([1; 32], [2; 32], &changed).is_err());
        assert!(seal_history([1; 32], [2; 32], &serde_json::json!({})).is_err());
        assert!(seal_history([1; 32], [2; 32], &serde_json::json!(vec![0; 1025])).is_err());
    }
}
