# Quote-generation boundary

`tdx_guest.rs` loads only the SHA256-pinned, absolute-path official
`libtdx_attest.so` from the measured initramfs. An open descriptor pins the
checked inode across `dlopen`. Dependencies are also inside the measured UKI.
The source revision is fixed in `inputs.json`; `abi_check.c` compiles against its
actual `tdx_attest.h` and checks struct sizes, enum size and function signatures.
The existing separate wallet QVL ABI review remains in the experiment's native/
folder. The production activation gate is unchanged.

`tdx_att_get_quote` receives 64 bytes of guest-computed REPORTDATA, a null key list
with count zero, null optional selected-key output, valid exclusive quote pointer
and size outputs, and flags zero. The pointer is copied only after success,
non-null and a 1 MiB bound. Intel's `tdx_att_free_quote` releases it on all result
paths, including failed parsing. Function pointers never outlive their library.
A successful generator call is **not signature verification**. TDX format,
REPORTDATA equality and debug-bit checks catch malformed generation locally;
wallet-local pinned native Intel QVL remains mandatory for acceptance.

The quote HTTP request contains only a nonzero challenge and enrollment/live
selection. A caller cannot choose REPORTDATA, executor keys, a verification result
or an accepted measurement. Enrollment binds the public keys, index, software
identity, custodian keys and random boot nonce. Live quotes reuse M23's binding
of committee, genesis, index, member, software+witness policy and challenge.
The private-key generation and quote generator live in the same measured guest
process. Its runtime, kernel, firmware and every executable library are in the
TCB. Independent MRTD/RTMR approval must cover the full UKI and boot options.

Restore capsules are encrypted under the new boot's guest-generated transport
key, authorized by the immutable per-executor operator public key, and bound to
its fresh boot nonce. Restored keys must match that executor's pinned committee
slot. No local-file/genesis fallback can reset an initialized witness. An
operator signature on the exact encrypted recovery capsule's digest confirms
that it was saved durably before the guest releases any vote. The custodian is a
trusted recovery authority, not a hardware-backed key service: it can recover
executor keys and read that executor's ledger. Do not conflate encrypted backups
with exclusive TEE custody. Use separate custodians and witness administrators.
