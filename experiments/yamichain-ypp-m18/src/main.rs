//! Measures real SHA-256 state authentication at registry scale, not 500k payments.
use rand::{rngs::StdRng, SeedableRng};
use std::time::Instant;
use yamichain_ypp_m18::*;
fn main() {
    let n = 500_000usize;
    let c = 64;
    let q = 4;
    let start = Instant::now();
    let mut values = (0..n)
        .map(|i| hash(b"synthetic-record-id", &[&(i as u64).to_le_bytes()]))
        .collect::<Vec<_>>();
    let mut tree = Tree::new(&values);
    let build_ms = start.elapsed().as_secs_f64() * 1000.;
    let mut rng = StdRng::seed_from_u64(18);
    let start = Instant::now();
    let mut selections = Vec::with_capacity(1000);
    for _ in 0..1000 {
        selections.push(select(n, c, q, 17, 300_017, &mut rng));
    }
    let select_us = start.elapsed().as_secs_f64() * 1e6 / 1000.;
    let chosen = &selections[0];
    let before = tree.root();
    let start = Instant::now();
    for i in chosen {
        values[*i] = hash(b"synthetic-update", &[&values[*i]]);
        tree.update(*i, values[*i]);
    }
    let update_ms = start.elapsed().as_secs_f64() * 1000.;
    let start = Instant::now();
    let path = tree.path(300_017);
    assert!(path.verify(tree.root(), values[300_017]));
    let path_verify_us = start.elapsed().as_secs_f64() * 1e6;
    assert_ne!(before, tree.root());
    let rss = std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|s| s.starts_with("VmHWM:"))
        .unwrap()
        .to_owned();
    println!(
        "{}",
        serde_json::json!({"registry_slots":n,"cohort_width":c,"cover_cohorts":q,"touched_slots":chosen.len(),"selection_samples":1000,"selection_mean_us":select_us,"tree_build_ms":build_ms,"tree_storage_bytes":tree.storage_bytes(),"sparse_tree_update_ms":update_ms,"account_path_bytes":path.siblings.len()*32,"path_build_verify_us":path_verify_us,"memory_high_water":rss,"synthetic_record_values":true,"registered_curve_keys_at_500k":false,"proof_at_500k":false,"privacy_verdict":"rejected"})
    );
}
