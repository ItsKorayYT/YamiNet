fn main() {
    let dir = std::env::args_os()
        .nth(1)
        .expect("corpus directory required");
    std::fs::create_dir_all(&dir).unwrap();
    for (index, seed) in yami_chain_consensus::fuzz_seeds().iter().enumerate() {
        std::fs::write(
            std::path::Path::new(&dir).join(format!("seed-{index}")),
            seed,
        )
        .unwrap();
    }
}
