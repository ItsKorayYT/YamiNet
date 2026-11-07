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
