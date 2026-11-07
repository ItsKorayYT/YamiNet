#!/bin/sh
set -eu
# Outputs only official Intel sources at the recorded commit. No custom quote acceptance code.
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
SOURCE=${1:?usage: build-intel.sh SOURCE_DIRECTORY}
PIN=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["intel_dcap_commit"])' "$ROOT/deployment/tdx/inputs.json")
if [ ! -d "$SOURCE/.git" ]; then
 git init "$SOURCE"
 git -C "$SOURCE" remote add origin https://github.com/intel/SGXDataCenterAttestationPrimitives.git
 git -C "$SOURCE" fetch --depth 1 origin "$PIN"
 git -C "$SOURCE" checkout --detach FETCH_HEAD
fi
[ "$(git -C "$SOURCE" rev-parse HEAD)" = "$PIN" ]
git -C "$SOURCE" diff --exit-code
git -C "$SOURCE" diff --cached --exit-code
make -C "$SOURCE/QuoteGeneration/quote_wrapper/tdx_attest/linux" -B -j2
printf '%s\n' "$SOURCE/QuoteGeneration/quote_wrapper/tdx_attest/linux/libtdx_attest.so"
