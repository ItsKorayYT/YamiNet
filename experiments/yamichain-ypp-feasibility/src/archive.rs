//! Public authenticated anchor archive; does not refresh PRIVATE vault witnesses.
//! An authoritative checkpoint must come from independently validated/finalized
//! ledger history. Membership alone never licenses a stale state-root overwrite.
use crate::merkle::{hash, Digest};
#[derive(Default)]
pub struct Archive {
    levels: Vec<Vec<Digest>>,
    empty: Vec<Digest>,
    count: usize,
}
#[derive(Clone)]
pub struct Membership {
    pub index: usize,
    pub count: usize,
    pub anchor: Digest,
    pub siblings: Vec<Digest>,
}
fn leaf(a: Digest) -> Digest {
    hash(&[b"YPP3-anchor-leaf", &a])
}
fn parent(a: Digest, b: Digest) -> Digest {
    hash(&[b"YPP3-anchor-node", &a, &b])
}
fn depth(n: usize) -> usize {
    if n <= 1 {
        0
    } else {
        (usize::BITS - (n - 1).leading_zeros()) as usize
    }
}
fn checkpoint(count: usize, root: Digest) -> Digest {
    hash(&[
        b"YPP3-anchor-checkpoint",
        &(count as u64).to_le_bytes(),
        &root,
    ])
}
impl Archive {
    pub fn new() -> Self {
        Self {
            levels: vec![Vec::new()],
            empty: vec![hash(&[b"YPP3-anchor-empty"])],
            count: 0,
        }
    }
    pub fn append(&mut self, a: Digest) {
        if self.levels.is_empty() {
            *self = Self::new();
        }
        let mut i = self.count;
        self.count += 1;
        self.levels[0].push(leaf(a));
        for level in 0..depth(self.count) {
            if self.empty.len() <= level + 1 {
                self.empty
                    .push(parent(self.empty[level], self.empty[level]));
            }
            if self.levels.len() <= level + 1 {
                self.levels.push(Vec::new());
            }
            let j = i & !1;
            let left = self.levels[level][j];
            let right = self.levels[level]
                .get(j + 1)
                .copied()
                .unwrap_or(self.empty[level]);
            let p = parent(left, right);
            i /= 2;
            if i < self.levels[level + 1].len() {
                self.levels[level + 1][i] = p;
            } else {
                self.levels[level + 1].push(p);
            }
        }
    }
    pub fn checkpoint(&self) -> Digest {
        if self.count == 0 {
            checkpoint(0, hash(&[b"YPP3-anchor-empty"]))
        } else {
            checkpoint(self.count, self.levels[depth(self.count)][0])
        }
    }
    pub fn proof(&self, index: usize, anchor: Digest) -> Option<Membership> {
        if index >= self.count || self.levels[0][index] != leaf(anchor) {
            return None;
        }
        let mut i = index;
        let siblings = (0..depth(self.count))
            .map(|level| {
                let s = self.levels[level]
                    .get(i ^ 1)
                    .copied()
                    .unwrap_or(self.empty[level]);
                i /= 2;
                s
            })
            .collect();
        Some(Membership {
            index,
            count: self.count,
            anchor,
            siblings,
        })
    }
    pub fn verify(checkpoint_root: Digest, p: &Membership) -> bool {
        if p.index >= p.count || p.siblings.len() != depth(p.count) {
            return false;
        }
        let mut n = leaf(p.anchor);
        let mut i = p.index;
        for s in &p.siblings {
            n = if i & 1 == 0 {
                parent(n, *s)
            } else {
                parent(*s, n)
            };
            i /= 2;
        }
        checkpoint(p.count, n) == checkpoint_root
    }
    pub fn logical_hash_bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum::<usize>() * 32
    }
}
