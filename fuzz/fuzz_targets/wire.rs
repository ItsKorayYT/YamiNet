#![no_main]
libfuzzer_sys::fuzz_target!(|data:&[u8]| {yami_chain_consensus::fuzz_decode(data);});
