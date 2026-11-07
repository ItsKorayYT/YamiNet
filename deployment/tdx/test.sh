#!/bin/sh
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
CARGO=${CARGO:-cargo}
: "${YAMI_QVL_LIBRARY:?pin the existing official native QVL build}"
: "${YAMI_QVL_SHA256:?pin its approved SHA256}"
: "${YAMI_INTEL_DCAP:?path to the pinned Intel quote-generation source}"
"$CARGO" test --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
 --features tdx-guest-devnet --lib tdx_guest
"$CARGO" test --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
 --features tdx-guest-devnet --test attestation_native -- --ignored
"$CARGO" test --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
 --features tdx-guest-devnet --lib native_wallet_rounds -- --ignored
"$CARGO" test -p yami-chain-privacy --test non_test_boundary
"$CARGO" clippy --locked --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
 --features tdx-guest-devnet --all-targets -- -D warnings
cc -Werror -fsyntax-only deployment/tdx/abi_check.c \
 -I"$YAMI_INTEL_DCAP/QuoteGeneration/quote_wrapper/tdx_attest"
python3 -m py_compile deployment/tdx/*.py scripts/yamichain-tdx-wallet
for script in deployment/tdx/*.sh deployment/tdx/init deployment/tdx/dhcp; do sh -n "$script"; done
if [ -n "${YAMI_TDX_GUEST_BUILD:-}" ]; then
 python3 deployment/tdx/validate-package.py "$YAMI_TDX_GUEST_BUILD"
fi
