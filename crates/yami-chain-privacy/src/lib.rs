//! Backend-neutral intent boundary. No proof construction or accepted proof version exists.
//! These are application intents, deliberately without a consensus wire encoding.

/// Public context that a future proof MUST bind in its entirety.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingContext {
    pub chain_id: String,
    pub protocol_version: u16,
    pub intent_hash: [u8; 32],
    pub finalized_anchor: [u8; 32],
    pub expiry_height: u64,
    pub fee_epoch: u64,
    pub public_fee: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedOutput {
    pub backend_public_descriptor: Vec<u8>,
    pub recipient_ciphertext: Vec<u8>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OpaqueProof {
    pub version: u16,
    pub bytes: Vec<u8>,
}
// Avoid copying large, untrusted opaque payloads into logs.
impl std::fmt::Debug for OpaqueProof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpaqueProof")
            .field("version", &self.version)
            .field("length", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfidentialTransfer {
    pub binding: BindingContext,
    pub backend_public_inputs: Vec<u8>,
    pub encrypted_outputs: Vec<EncryptedOutput>,
    pub proof: OpaqueProof,
}

/// Future module-owned, public effect. Payload semantics need a reviewed crypto ADR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfidentialProtocolAction {
    pub transfer: ConfidentialTransfer,
    pub module: u16,
    pub public_effect: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationError {
    UnsupportedProofVersion(u16),
}

/// Non-constructible evidence reserves the return contract without authorizing value.
#[derive(Debug)]
pub struct VerifiedEffects {
    _private: (),
}

/// Concrete sealed boundary: no replaceable accepting implementation in execution.
#[derive(Clone, Copy, Debug, Default)]
pub struct PrivacyVerifier;
impl PrivacyVerifier {
    pub fn verify_payment(
        &self,
        intent: &ConfidentialTransfer,
    ) -> Result<VerifiedEffects, VerificationError> {
        Err(VerificationError::UnsupportedProofVersion(
            intent.proof.version,
        ))
    }

    pub fn verify_protocol_action(
        &self,
        intent: &ConfidentialProtocolAction,
    ) -> Result<VerifiedEffects, VerificationError> {
        self.verify_payment(&intent.transfer)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    // Independent transparent mathematical model; never a consensus verifier.
    fn conserve(inputs: &[u128], outputs: &[u128], fee: u128) -> bool {
        let input = inputs.iter().try_fold(0_u128, |a, b| a.checked_add(*b));
        let output = outputs.iter().try_fold(fee, |a, b| a.checked_add(*b));
        input.is_some() && input == output
    }

    #[test]
    fn bounded_conservation_and_unique_consumption_model() {
        for value in 0..1024_u128 {
            assert!(conserve(&[value, 7], &[value, 5], 2));
            assert!(!conserve(&[value, 7], &[value, 6], 2));
        }
        assert!(!conserve(&[u128::MAX, 1], &[0], 0));
        assert!(!conserve(&[0], &[u128::MAX], 1));
        let mut spent = BTreeSet::new();
        assert!(spent.insert([1_u8; 32]));
        assert!(!spent.insert([1_u8; 32]));
        // This tests a model, not a construction ensuring immutable spend tags.
    }
}
