//! Socket-free consent/replay prerequisite for the documented QUIC proposal.
//! No UDP sockets, endpoint discovery, punching, or document transport is shipped.
use crate::invalid;
use std::{io, net::SocketAddrV4};
use yami_identity::{NodeId, RendezvousConsent, Signature};
pub const MAX_SESSIONS: usize = 64;
pub const CONSENT_TTL: u64 = 30;
pub struct ConsentGate {
    coordinator: NodeId,
    used: Vec<([u8; 32], u64)>,
    last_time: u64,
}
impl ConsentGate {
    pub fn new(coordinator: NodeId) -> Self {
        Self {
            coordinator,
            used: Vec::new(),
            last_time: 0,
        }
    }
    pub fn authorize(
        &mut self,
        consent: &RendezvousConsent,
        initiator: &Signature,
        responder: &Signature,
        at: u64,
    ) -> io::Result<()> {
        if at < self.last_time {
            return Err(invalid("rendezvous clock rollback"));
        }
        self.last_time = at;
        if consent.coordinator != self.coordinator
            || consent.initiator == consent.responder
            || consent.coordinator == consent.initiator
            || consent.coordinator == consent.responder
            || consent.nonce == [0; 32]
            || consent.expires <= at
            || consent.expires > at.saturating_add(CONSENT_TTL)
        {
            return Err(invalid("invalid rendezvous parties, nonce or expiry"));
        }
        self.used.retain(|(_, expires)| *expires > at);
        if self.used.len() >= MAX_SESSIONS
            || self.used.iter().any(|(nonce, _)| *nonce == consent.nonce)
        {
            return Err(invalid("rendezvous replay or capacity limit"));
        }
        consent
            .initiator
            .verify(consent, initiator)
            .map_err(io::Error::other)?;
        consent
            .responder
            .verify(consent, responder)
            .map_err(io::Error::other)?;
        self.used.push((consent.nonce, consent.expires));
        Ok(())
    }
}
/// Future observed UDP endpoints must pass public policy; this does not turn a
/// caller-supplied value into proof of observation or of TCP/QUIC reachability.
pub fn validate_candidate(endpoint: SocketAddrV4) -> io::Result<()> {
    if endpoint.port() == 0 || !ynp_tls::is_public_ipv4(*endpoint.ip()) {
        return Err(invalid("rendezvous candidate is not public unicast IPv4"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use yami_identity::NodeIdentity;
    #[test]
    fn mutual_consent_binds_coordinator_participants_nonce_and_expiry_and_rejects_replay() {
        let (r, a, b) = (
            NodeIdentity::generate().unwrap(),
            NodeIdentity::generate().unwrap(),
            NodeIdentity::generate().unwrap(),
        );
        let consent = RendezvousConsent {
            coordinator: *r.id(),
            initiator: *a.id(),
            responder: *b.id(),
            nonce: [1; 32],
            expires: 120,
        };
        let signatures = (a.sign(&consent), b.sign(&consent));
        let mut gate = ConsentGate::new(*r.id());
        assert!(gate
            .authorize(&consent, &signatures.0, &signatures.0, 100)
            .is_err());
        for field in 0..4 {
            let mut changed = consent.clone();
            match field {
                0 => changed.coordinator = *a.id(),
                1 => changed.responder = *r.id(),
                2 => changed.nonce[0] ^= 1,
                _ => changed.expires += 1,
            };
            assert!(gate
                .authorize(&changed, &signatures.0, &signatures.1, 100)
                .is_err());
        }
        gate.authorize(&consent, &signatures.0, &signatures.1, 100)
            .unwrap();
        assert!(gate
            .authorize(&consent, &signatures.0, &signatures.1, 100)
            .is_err());
        assert!(gate
            .authorize(&consent, &signatures.0, &signatures.1, 99)
            .is_err());
        assert!(gate
            .authorize(&consent, &signatures.0, &signatures.1, 120)
            .is_err());
    }
    #[test]
    fn consent_slots_and_public_candidate_policy_are_bounded() {
        let (r, a, b) = (
            NodeIdentity::generate().unwrap(),
            NodeIdentity::generate().unwrap(),
            NodeIdentity::generate().unwrap(),
        );
        let mut gate = ConsentGate::new(*r.id());
        for i in 1..=MAX_SESSIONS + 1 {
            let consent = RendezvousConsent {
                coordinator: *r.id(),
                initiator: *a.id(),
                responder: *b.id(),
                nonce: [i as u8; 32],
                expires: 120,
            };
            let result = gate.authorize(&consent, &a.sign(&consent), &b.sign(&consent), 100);
            assert_eq!(result.is_ok(), i <= MAX_SESSIONS);
        }
        for address in [
            "127.0.0.1:1",
            "10.0.0.1:1",
            "100.64.0.1:1",
            "169.254.1.1:1",
            "198.51.100.1:1",
            "224.0.0.1:1",
            "8.8.8.8:0",
        ] {
            assert!(validate_candidate(address.parse().unwrap()).is_err());
        }
        validate_candidate("8.8.8.8:7443".parse().unwrap()).unwrap(); // Policy only; sends nothing.
    }
}
