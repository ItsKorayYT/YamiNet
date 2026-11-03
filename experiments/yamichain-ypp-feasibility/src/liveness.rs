//! Deterministic event simulation using measured latency samples. It does NOT
//! claim native parallel-prover throughput or make unsafe writes safe.
#[derive(serde::Serialize)]
pub struct Outcome {
    pub clients: usize,
    pub background_roots_per_second: f64,
    pub retained_roots: Option<u64>,
    pub accepted: usize,
    pub expired: usize,
    pub last_service_ms: f64,
    pub simulated_history_height: u64,
}
pub fn simulate(
    proof_ms: &[f64],
    verify_ms: f64,
    clients: usize,
    rate: f64,
    window: Option<u64>,
) -> Outcome {
    assert!(
        !proof_ms.is_empty()
            && proof_ms.iter().all(|n| n.is_finite() && *n >= 0.0)
            && verify_ms.is_finite()
            && verify_ms >= 0.0
            && rate.is_finite()
            && rate >= 0.0
    );
    let mut done = (0..clients)
        .map(|i| proof_ms[i % proof_ms.len()])
        .collect::<Vec<_>>();
    done.sort_by(f64::total_cmp);
    let mut accepted = 0;
    let mut expired = 0;
    let mut free = 0.0_f64;
    let mut height = 0;
    for ready in done {
        free = free.max(ready) + verify_ms;
        height = (free * rate / 1000.).floor() as u64 + accepted as u64;
        if window.is_none_or(|n| height < n) {
            accepted += 1;
            height += 1;
        } else {
            expired += 1;
        }
    }
    Outcome {
        clients,
        background_roots_per_second: rate,
        retained_roots: window,
        accepted,
        expired,
        last_service_ms: free,
        simulated_history_height: height,
    }
}
pub fn grid(proof_ms: &[f64], verify_ms: f64) -> Vec<Outcome> {
    let mut out = Vec::new();
    for clients in [2, 8, 64] {
        for rate in [0., 1., 4.] {
            for window in [Some(4), Some(64), Some(4096), None] {
                out.push(simulate(proof_ms, verify_ms, clients, rate, window));
            }
        }
    }
    out
}
