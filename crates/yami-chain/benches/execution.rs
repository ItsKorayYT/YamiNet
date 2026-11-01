//! Non-financial execution only. No proof verification or networking measurements.
use std::{hint::black_box, time::Instant};
use yami_chain::{Genesis, State};
fn main() {
    let mut state = State::new(Genesis::local()).unwrap();
    let start = Instant::now();
    let blocks = 1000;
    for _ in 0..blocks {
        let block = state.development_block().unwrap();
        state = state.execute(black_box(&block)).unwrap();
        black_box(state.root());
    }
    println!(
        "nonfinancial_blocks={blocks} elapsed_ms={} state_root={}",
        start.elapsed().as_millis(),
        state.root()
    );
}
