//! Inactive offline fixture reader. The exact target anchor is independently
//! recorded by the signed-history campaign, never copied from a peer manifest.
use std::{path::Path, time::Instant};
use yami_chain::{checkpoint_v2::StateAnchor, Digest, Genesis};
use yami_chain_consensus::checkpoint_v2::{verify_context, MAX_ENVELOPE_BYTES};
fn digest(text: &str) -> eyre::Result<Digest> {
    eyre::ensure!(text.len() == 64 && text.is_ascii(), "digest encoding");
    let mut bytes = [0; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)?;
    }
    Ok(Digest(bytes))
}
fn main() -> eyre::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    eyre::ensure!(args.len() == 5, "checkpoint-v2-reader FIXTURE_DIR ENTRIES HEIGHT TIP ROOT; independent exact target required");
    let entries: usize = args[1].parse()?;
    eyre::ensure!(
        [1024, 100_000, 250_000, 500_000].contains(&entries),
        "fixture size"
    );
    // Matches only the existing DISTINCT-account signed fixture domains.
    // This executable cannot select production or isolated-network identities.
    let genesis = Genesis::development_schedule(&[(1, 4), (800_001 + entries as u64 / 10, 5)])?;
    let minimum = StateAnchor {
        genesis: genesis.id(),
        height: args[2].parse()?,
        tip: digest(&args[3])?,
        root: digest(&args[4])?,
    };
    let path = Path::new(&args[0]);
    let bytes =
        yami_chain_storage::read_bounded(&path.join("inactive-envelope.bin"), MAX_ENVELOPE_BYTES)?;
    let at = Instant::now();
    let context = verify_context(&genesis, minimum, &bytes, std::iter::empty::<&[u8]>())?;
    let finality = at.elapsed().as_secs_f64();
    let at = Instant::now();
    let state = context.recover(&path.join("inactive-v2"))?;
    let recovery = at.elapsed().as_secs_f64();
    eyre::ensure!(
        state.names() == entries && state.accounts() == entries && state.anchor() == minimum,
        "fixture/independent anchor mismatch"
    );
    println!("{{\"entries\":{entries},\"height\":{},\"root\":\"{}\",\"finality_seconds\":{finality},\"recovery_seconds\":{recovery},\"production_active\":false}}", minimum.height, minimum.root);
    Ok(())
}
