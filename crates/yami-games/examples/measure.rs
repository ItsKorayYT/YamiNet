//! Headless VM benchmark. UI/presentation measurements are separate.
use std::{path::Path, time::Instant};
use yami_games::*;
fn main() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Pass a local game manifest")?;
    let start = Instant::now();
    let (_, bytes) = load_local(Path::new(&path))?;
    let mut vm = Runtime::new(&bytes)?;
    let load = start.elapsed();
    vm.frame(0, START)?;
    vm.frame(0, START)?;
    let mut samples = Vec::new();
    let mut commands = 0;
    for i in 0..10_000 {
        let now = Instant::now();
        let frame = vm.frame(
            if i % 240 < 120 { LEFT } else { RIGHT },
            if i % 240 == 0 { START } else { 0 },
        )?;
        samples.push(now.elapsed().as_nanos());
        commands = commands.max(frame.commands.len());
    }
    samples.sort_unstable();
    let avg = samples.iter().sum::<u128>() as f64 / samples.len() as f64 / 1_000_000.;
    println!("load_ms={:.3} frames={} avg_ms={avg:.4} p50_ms={:.4} p95_ms={:.4} p99_ms={:.4} max_ms={:.4} max_commands={commands}",load.as_secs_f64()*1000.,samples.len(),samples[5000] as f64/1e6,samples[9500] as f64/1e6,samples[9900] as f64/1e6,samples[9999] as f64/1e6);
    Ok(())
}
