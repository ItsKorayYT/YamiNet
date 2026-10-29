use crate::{invalid, Network};
use std::{io, net::SocketAddrV4};
use yami_identity::{sha256, NodeId};
pub const BUCKET_SIZE: usize = 4;
pub const MAX_ROUTES: usize = 128;
pub const ROUTE_TTL: u64 = 15 * 60;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Contact {
    pub id: NodeId,
    pub endpoint: SocketAddrV4,
}
impl Contact {
    pub fn new(id: NodeId, endpoint: SocketAddrV4, network: Network) -> io::Result<Self> {
        network.check(endpoint)?;
        Ok(Self { id, endpoint })
    }
    pub fn validate(self, network: Network) -> io::Result<Self> {
        Self::new(self.id, self.endpoint, network)
    }
}
pub fn distance(one: NodeId, two: NodeId) -> [u8; 32] {
    let a = sha256(&one.public_key());
    let b = sha256(&two.public_key());
    std::array::from_fn(|i| a[i] ^ b[i])
}
pub(crate) fn bucket(one: NodeId, two: NodeId) -> Option<usize> {
    let d = distance(one, two);
    d.iter()
        .position(|byte| *byte != 0)
        .map(|i| i * 8 + d[i].leading_zeros() as usize)
}
#[derive(Clone)]
pub struct RoutingTable {
    local: NodeId,
    contacts: Vec<(Contact, u64)>,
}
impl RoutingTable {
    pub fn new(local: NodeId) -> Self {
        Self {
            local,
            contacts: Vec::new(),
        }
    }
    pub fn contacts(&self) -> Vec<Contact> {
        self.contacts.iter().map(|(c, _)| *c).collect()
    }
    pub fn expire(&mut self, now: u64) {
        self.contacts
            .retain(|(_, seen)| *seen <= now && now - *seen < ROUTE_TTL);
    }
    /// Call only after an authenticated successful RPC, never on advertisement.
    pub(crate) fn observed(&mut self, contact: Contact, now: u64) -> io::Result<()> {
        self.expire(now);
        let Some(index) = bucket(self.local, contact.id) else {
            return Ok(());
        };
        if let Some(old) = self.contacts.iter_mut().find(|(c, _)| c.id == contact.id) {
            *old = (contact, now);
            return Ok(());
        }
        if self.contacts.len() >= MAX_ROUTES
            || self
                .contacts
                .iter()
                .filter(|(c, _)| bucket(self.local, c.id) == Some(index))
                .count()
                >= BUCKET_SIZE
        {
            return Err(invalid(
                "routing bucket/table is full; live contacts are not displaced",
            ));
        }
        self.contacts.push((contact, now));
        Ok(())
    }
    pub(crate) fn failed(&mut self, contact: Contact) {
        self.contacts.retain(|(c, _)| *c != contact);
    }
}
