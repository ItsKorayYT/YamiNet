# M22 native Intel QVL boundary review

This is an implementation review record, not an independent security audit.
Supported platform: 64-bit Linux, trusted wallet OS, OpenSSL 3.

Intel source: `intel/SGX-TDX-DCAP-QuoteVerificationLibrary`, pinned commit
`d12717e3f1f2ab81313f88001fd902b5ef5f9c8c` (official Intel DCAP QVL).
Reviewed interface: `Src/AttestationLibrary/include/SgxEcdsaAttestation/QuoteVerification.h`,
SHA-256 `9cf24c1315a436a432e6c2a8b22292ec435ae07bfc92db92cf1c7dd33cd059cc`.
The linked C++ verification implementation is unmodified. Custom CMake compiles
its core sources directly against system OpenSSL without Intel's Hunter download
or SGX/QvE prerequisites. `abi_check.cpp` compiles exact function-type assertions
against that official header and asserts Status/time_t sizes.

| Function | Rust ABI / lifetime obligation | Decision |
| --- | --- | --- |
| GetQECertificationDataSize | immutable bounded byte slice, u32 length, live exclusive u32 output | require STATUS_OK and 1..1MiB size |
| GetQECertificationData | same immutable slice, exactly sized mutable Vec, u16 output | require STATUS_OK and type 5 PCK chain; reject interior NULs |
| VerifyPCKCertificate | live chain/root CStrings; two-element array of live CRL CStrings; non-null native time_t | require STATUS_OK; authenticate chain and both revocation lists to compiled Intel root |
| VerifyTCBInfo | live signed JSON, issuer chain, root CRL, pinned root, non-null time_t | require STATUS_OK, including authenticated expiry checks |
| VerifyEnclaveIdentity | same lifetimes, signed QE identity JSON required, never null/optional | require STATUS_OK |
| VerifyQuote | live bounded quote and authenticated PCK/CRL/TCB/QE CStrings | require STATUS_OK, no warning/status downgrade |

All outputs have allocated capacities before calls. All inputs remain alive across
synchronous native calls. Intel does not retain caller buffers. Rust owns and
frees all caller buffers; the API does not return native-owned allocation pointers.
Status is a 32-bit C enum represented by c_int; time_t uses libc's native type.
No Rust references or Rust enums cross the ABI. CString rejects embedded NULs.
A trailing Intel-provided terminator is stripped once before CString conversion.

Only the private adapter contains unsafe operations. Function pointers are copied
from required exact symbols and kept with their owning Library. The checked file
FD is retained too, avoiding reused /proc/self/fd loader names while alive. The
wallet supplies an absolute native-library path and independently trusted SHA-256
pin. The pin is checked before dlopen; there is no system-library fallback, host
verdict input, executable helper result or mock native backend. Library inode and
its dependencies must remain immutable under the trusted wallet OS. An attacker
who can modify wallet-native code/dependencies or its configuration defeats this
boundary. Hash pinning is not sandboxing or a supply-chain audit.

An authenticated root PEM is compiled into Rust, never selected from supplied
collateral. Its P-256 public point was compared to Intel DCAP's INTEL_ROOT_PUB_KEY
in `ae/QvE/qve/qve.cpp` at commit
`72f800e9696f382464bc8ef243311edf8bd7dc13`; all 65 bytes match.
Certificate DER SHA-256:
`44a0196b2b99f889b8e149e95b807a350e7424964399e885a7cbb8ccfab674d3`.
SubjectPublicKeyInfo DER SHA-256:
`a0af031289f5d5d4132f9186068a7fc13628633ba235777472e29b6b6c67a49e`.

The complete verification sequence follows Intel's AttestationApp combination of
PCK, TCB, QE identity and quote checks. Calling VerifyQuote alone is insufficient:
the authentic archived fixture still returns quote status 0 after its collateral
expires, while the PCK/CRL stage returns 79. Rust additionally enforces notBefore /
thisUpdate / issueDate and notAfter / nextUpdate at the same verification time.
Only after all stages does it mint a non-deserializable VerifiedQuote. Public
historical diagnostics cannot mint WalletRequest or replace live clock/challenges.

Native tests cover authentic success at archived time, expiry, authentic unsupported
TCB, altered quote/signatures, altered signed collateral/CRLs, bounds, strict
framing and wallet rejection. No authentic revoked-PCK or authentic debug-TD fixture
was found: those exact positive-signature negative cases remain unexercised. There
are no generated keys, custom test roots or invented passing quotes in this path.
Malicious native-library replacement is rejected by pin; a missing library/symbol
cannot result in acceptance. Native implementation/dependencies still require an
independent audit before activation, including robustness against hostile inputs.
