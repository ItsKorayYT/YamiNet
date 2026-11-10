# Development economic policy versions

Economic policies and signed fee authorizations have an explicit version. Missing
versions mean legacy version 1, whose canonical bytes remain identical. Every
other 16-bit version is rejected, including 0; the wallet also refuses unsupported
fee policy versions. No network response can select a newer execution algorithm.
This is fail-closed versioning, not an authorized policy upgrade mechanism.
Activating a new version still requires reviewed rules and an explicit upgrade
protocol; changing the version in JSON cannot activate it.

Validation: `cargo test --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml --test economics` includes rejection of every unsupported 16-bit policy version, a signed unsupported authorization, and unchanged legacy canonical bytes. Strict Clippy with all targets and features passes. This does not change the independent production financial-proof version gates.
