use crate::{crypto::*, Error, Result};
use ark_bls12_381::Fr;
use ark_crypto_primitives::crh::CRHScheme;
use std::sync::Arc;
pub const DEPTH: usize = 5;
pub const CAPACITY: usize = 1 << DEPTH;
pub const ROOT_WINDOW: usize = 4;
#[derive(Clone)]
pub(crate) struct Tree {
    pub(crate) params: Arc<TreeParams>,
    pub(crate) leaves: Vec<Digest>,
}
#[derive(Clone)]
pub(crate) struct Witness {
    pub index: usize,
    pub siblings: [Digest; DEPTH],
}
pub(crate) fn parent(p: &TreeParams, l: Digest, r: Digest) -> Fr {
    TreeHash::evaluate(p, [&[9][..], &l, &r].concat().as_slice()).expect("fixed hash input")
}
impl Tree {
    pub fn new(params: Arc<TreeParams>) -> Self {
        Self {
            params,
            leaves: Vec::new(),
        }
    }
    pub(crate) fn len(&self) -> usize {
        self.leaves.len()
    }
    pub(crate) fn append(&mut self, c: Digest) -> Result<()> {
        if self.leaves.len() >= CAPACITY {
            return Err(Error::Capacity);
        }
        self.leaves.push(c);
        Ok(())
    }
    fn layers(&self) -> Vec<Vec<Digest>> {
        let mut leaves = self.leaves.clone();
        leaves.resize(CAPACITY, [0; 32]);
        let mut layers = vec![leaves];
        for i in 0..DEPTH {
            let next = layers[i]
                .chunks_exact(2)
                .map(|v| {
                    bytes(&parent(&self.params, v[0], v[1]))
                        .try_into()
                        .expect("field")
                })
                .collect();
            layers.push(next);
        }
        layers
    }
    pub fn root(&self) -> Fr {
        let b = self.layers()[DEPTH][0];
        Fr::from_le_bytes_mod_order(&b)
    }
    pub(crate) fn witness(&self, index: usize) -> Result<Witness> {
        if index >= self.leaves.len() {
            return Err(Error::Membership);
        }
        let layers = self.layers();
        let mut j = index;
        let siblings = std::array::from_fn(|level| {
            let v = layers[level][j ^ 1];
            j /= 2;
            v
        });
        Ok(Witness { index, siblings })
    }
    pub(crate) fn locate(&self, c: Digest) -> Result<Witness> {
        let i = self
            .leaves
            .iter()
            .position(|x| *x == c)
            .ok_or(Error::Membership)?;
        self.witness(i)
    }
}
use ark_ff::PrimeField;
