//! Rejected-candidate selection and standard authenticated-state primitives.
//! No payment backend or new proof system is exported.
use rand::{seq::index::sample, Rng};
use sha2::{Digest, Sha256};
pub type Hash = [u8; 32];
pub fn hash(domain: &[u8], parts: &[&[u8]]) -> Hash {
    let mut h = Sha256::new();
    h.update((domain.len() as u64).to_le_bytes());
    h.update(domain);
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().into()
}
/// Uniform cover cohorts, conditioned on including both real cohorts.
/// This unavoidable conditioning leaks participants; this candidate is rejected.
pub fn select<R: Rng + ?Sized>(
    n: usize,
    width: usize,
    count: usize,
    sender: usize,
    recipient: usize,
    rng: &mut R,
) -> Vec<usize> {
    assert!(width > 0 && sender < n && recipient < n && sender != recipient);
    let groups = n.div_ceil(width);
    let a = sender / width;
    let b = recipient / width;
    let mut chosen = vec![a];
    if a != b {
        chosen.push(b);
    }
    assert!(count >= chosen.len() && count <= groups);
    // Sample ranks among groups other than a,b: O(count) memory, no N-sized shuffle.
    let mut excluded = chosen.clone();
    excluded.sort_unstable();
    for rank in sample(rng, groups - excluded.len(), count - chosen.len()).iter() {
        let mut id = rank;
        for skip in &excluded {
            if id >= *skip {
                id += 1;
            }
        }
        chosen.push(id);
    }
    chosen.sort_unstable();
    chosen
        .into_iter()
        .flat_map(|id| id * width..((id + 1) * width).min(n))
        .collect()
}
fn leaf(index: usize, value: Hash) -> Hash {
    hash(b"leaf-v1", &[&(index as u64).to_le_bytes(), &value])
}
fn parent(a: Hash, b: Hash) -> Hash {
    hash(b"node-v1", &[&a, &b])
}
#[derive(Clone)]
pub struct Tree {
    n: usize,
    width: usize,
    nodes: Vec<Hash>,
}
#[derive(Clone)]
pub struct Path {
    pub n: usize,
    pub index: usize,
    pub siblings: Vec<Hash>,
}
impl Tree {
    pub fn new(values: &[Hash]) -> Self {
        assert!(!values.is_empty());
        let n = values.len();
        let width = n.next_power_of_two();
        let mut nodes = vec![[0; 32]; 2 * width];
        for i in 0..width {
            nodes[width + i] = if i < n {
                leaf(i, values[i])
            } else {
                hash(b"padding-v1", &[&(i as u64).to_le_bytes()])
            };
        }
        for i in (1..width).rev() {
            nodes[i] = parent(nodes[2 * i], nodes[2 * i + 1]);
        }
        Self { n, width, nodes }
    }
    pub fn root(&self) -> Hash {
        hash(
            b"root-v1",
            &[&(self.n as u64).to_le_bytes(), &self.nodes[1]],
        )
    }
    pub fn storage_bytes(&self) -> usize {
        self.nodes.len() * 32
    }
    pub fn update(&mut self, index: usize, value: Hash) {
        assert!(index < self.n);
        let mut i = self.width + index;
        self.nodes[i] = leaf(index, value);
        while i > 1 {
            i /= 2;
            self.nodes[i] = parent(self.nodes[2 * i], self.nodes[2 * i + 1]);
        }
    }
    pub fn path(&self, index: usize) -> Path {
        assert!(index < self.n);
        let mut i = self.width + index;
        let mut siblings = vec![];
        while i > 1 {
            siblings.push(self.nodes[i ^ 1]);
            i /= 2;
        }
        Path {
            n: self.n,
            index,
            siblings,
        }
    }
}
impl Path {
    pub fn verify(&self, root: Hash, value: Hash) -> bool {
        if self.n == 0 || self.index >= self.n || self.n.checked_next_power_of_two().is_none() {
            return false;
        }
        let width = self.n.next_power_of_two();
        if self.siblings.len() != width.ilog2() as usize {
            return false;
        }
        let mut i = self.index;
        let mut v = leaf(i, value);
        for sibling in &self.siblings {
            v = if i & 1 == 0 {
                parent(v, *sibling)
            } else {
                parent(*sibling, v)
            };
            i /= 2;
        }
        root == hash(b"root-v1", &[&(self.n as u64).to_le_bytes(), &v])
    }
}
/// External finality/freshness must authenticate this value. Hashing cannot do so.
pub fn checkpoint(epoch: u64, root: Hash) -> Hash {
    hash(b"checkpoint-v1", &[&epoch.to_le_bytes(), &root])
}
#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};
    #[test]
    fn selection_contains_endpoints_and_only_whole_cohorts() {
        let mut rng = StdRng::seed_from_u64(18);
        for n in [17, 64, 500_000] {
            for _ in 0..100 {
                let s = select(n, 8, 3, 0, n - 1, &mut rng);
                assert!(s.contains(&0) && s.contains(&(n - 1)));
                assert!(s.windows(2).all(|p| p[0] < p[1]));
                assert!(s.len() <= 24);
                let groups: std::collections::HashSet<_> = s.iter().map(|i| i / 8).collect();
                assert_eq!(groups.len(), 3);
            }
        }
    }
    #[test]
    fn merkle_tamper_replay_and_padding() {
        let mut values = (0u64..17)
            .map(|i| hash(b"test", &[&i.to_le_bytes()]))
            .collect::<Vec<_>>();
        let mut tree = Tree::new(&values);
        let old = tree.root();
        let p = tree.path(16);
        assert!(p.verify(old, values[16]));
        assert!(!p.verify(old, values[0]));
        let mut bad = p.clone();
        bad.n = 16;
        assert!(!bad.verify(old, values[16]));
        let mut bad = p.clone();
        bad.index = 15;
        assert!(!bad.verify(old, values[16]));
        let mut bad = p.clone();
        bad.siblings.pop();
        assert!(!bad.verify(old, values[16]));
        let old_value = values[16];
        values[16] = hash(b"changed", &[]);
        tree.update(16, values[16]);
        assert!(!p.verify(tree.root(), old_value));
        assert!(tree.path(16).verify(tree.root(), values[16]));
        assert_eq!(tree.root(), Tree::new(&values).root());
        assert_ne!(checkpoint(1, old), checkpoint(2, old));
    }
}
