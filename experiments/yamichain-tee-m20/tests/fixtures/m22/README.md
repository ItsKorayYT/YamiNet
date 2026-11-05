# Authentic public fixture provenance

Source: https://github.com/Phala-Network/dcap-qvl
Commit: `05c200b752ecb824b73e5b1ae63cc047f1702ab2`.
License: MIT, copied in PHALA-LICENSE. This is fixture data only: the implementation
uses Intel's official native QVL, not Phala's verifier.

- `sample/tdx_quote` was copied intact as `tdx_quote_upstream_padded` (5,006 bytes).
  Its declared v4 signature length is 4,300 bytes, so the canonical quote ends at
  byte 4,936. The remaining 70 bytes are all zero, outside the quote's declared
  length and signature. `tdx_quote` here is exactly the first 4,936 upstream bytes.
  No signed header/body, signature, certificate or Intel-signed collateral byte
  was edited, regenerated or re-signed. The original padded buffer is retained
  and explicitly rejected by the strict wrapper, although Intel's parser tolerates
  the unsigned buffer padding. The canonical quote passes native QVL at archived
  time 2025-06-20T00:00:00Z.
- `sample/tdx_quote_collateral.json` is copied unchanged. PCK CRL and signed TCB/QE
  collateral expire in July 2025; root CRL expires in April 2026. They must NOT
  authorize a present-day wallet. Live tests reject them using wallet system time.
- `sample/tdx_quote_outdated` and its collateral are copied byte-for-byte. This is
  an authentic v5 TD15 quote; native QVL returns STATUS_TCB_NOT_SUPPORTED (49) at
  2026-02-20T00:00:00Z, inside its collateral validity period. No non-OK result is
  promoted to success.

All source/canonical digests and normalization details are recorded in manifest.json.
These quotes attest public third-party systems, not YamiChain executor instances.
Their measurements are never installed as a production allowlist. Diagnostic tests
use matching fixture measurements only to isolate subsequent binding-policy checks.
They contain no YamiChain keys or fresh YamiChain wallet REPORTDATA. Historical
verification success is not a live attestation, hardware demonstration or payment.
