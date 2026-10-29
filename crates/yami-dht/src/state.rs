use crate::{distance, invalid, Contact, Network, PeerRecord, RoutingTable, MAX_REPLY_RECORDS};
use std::io;
use yami_identity::NodeId;
pub const MAX_CACHE: usize = 256;
/// Expired records are never used as locations. Their signed high-water marks
/// remain bounded and persistent so expiry/churn cannot bypass rollback checks.
/// At capacity, new IDs fail closed rather than evict anti-rollback history.
#[derive(Clone)]
pub struct State {
    pub local: NodeId,
    pub network: Network,
    pub routing: RoutingTable,
    pub(crate) records: Vec<PeerRecord>,
}
impl State {
    pub fn new(local: NodeId, network: Network) -> Self {
        Self {
            local,
            network,
            routing: RoutingTable::new(local),
            records: Vec::new(),
        }
    }
    pub fn records(&self) -> &[PeerRecord] {
        &self.records
    }
    pub fn get(&self, id: NodeId, now: u64) -> Option<PeerRecord> {
        self.records
            .iter()
            .find(|r| r.id() == id && r.live(now))
            .cloned()
    }
    pub fn accept(&mut self, record: PeerRecord, now: u64) -> io::Result<bool> {
        record.verify(self.network, now)?;
        if let Some(old) = self.records.iter_mut().find(|r| r.id() == record.id()) {
            if record.sequence() < old.sequence()
                || (record.sequence() == old.sequence() && *old != record)
            {
                return Err(invalid("peer record rollback/equivocation rejected"));
            }
            if *old == record {
                return Ok(false);
            }
            *old = record;
        } else {
            if self.records.len() >= MAX_CACHE {
                return Err(invalid("peer cache/high-water budget is full"));
            }
            self.records.push(record);
        }
        Ok(true)
    }
    pub(crate) fn closest(&self, target: NodeId, now: u64) -> Vec<PeerRecord> {
        let mut records: Vec<_> = self
            .records
            .iter()
            .filter(|r| r.live(now))
            .cloned()
            .collect();
        records.sort_by_key(|r| distance(r.id(), target));
        records.truncate(MAX_REPLY_RECORDS);
        // A joining peer needs a signed contact for its authenticated responder,
        // not only unsigned bootstrap pins or records near the lookup target.
        if !records.iter().any(|r| r.id() == self.local) {
            if let Some(own) = self.get(self.local, now) {
                records.truncate(MAX_REPLY_RECORDS - 1);
                records.push(own);
            }
        }
        records
    }
    pub(crate) fn candidates(&mut self, now: u64) -> Vec<Contact> {
        self.routing.expire(now);
        let mut contacts = self.routing.contacts();
        for record in self
            .records
            .iter()
            .filter(|r| r.live(now) && r.id() != self.local)
        {
            contacts.extend(record.endpoints().iter().map(|endpoint| Contact {
                id: record.id(),
                endpoint: *endpoint,
            }));
        }
        contacts
    }
}
