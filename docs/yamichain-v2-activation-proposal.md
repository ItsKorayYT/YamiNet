# Proposed v2 checkpoint activation — not approved or implemented

M14's chunk codec and M15's measurements are experiments. Neither constitutes a
production checkpoint protocol or permission to increase limits. All existing v1
limits, block/transaction/certificate encodings, flat state roots, signing safety,
rollback protections and disabled financial functionality remain in force.

## Authentication and trust boundary

An approved checkpoint envelope would carry an explicit codec/envelope version,
genesis ID, height, finalized block ID, state root, next-committee hash, exact
finalized record/certificate, manifest commitment, counts and total bytes.
Unknown versions and limits must fail before allocating application indexes.

The existing certificate authenticates its block ID. Verifying the complete
header binds that ID to height, parent, committee and state root under the pinned
genesis/schedule. It does **not** authenticate an arbitrary manifest digest or
prove ancestry from the receiver's independent minimum anchor by itself. The
receiver must retain its independently trusted genesis, committee schedule,
checkpoint/rollback minimum and any required authenticated ancestry. It must
reject a lower or conflicting checkpoint even when its certificate is valid.

There are two distinct transport trust models requiring an explicit choice:

1. **Root-authenticated completion.** Verify the existing finality certificate,
   state context and independent minimum first. Treat the peer's manifest/chunk
   commitments as untrusted transport framing. Enforce all budgets, reconstruct
   the entire state, validate every invariant and compute the unchanged flat root.
   Return/publish nothing until that root equals the independently authenticated
   header root. Chunk hashes alone provide no early Byzantine authenticity.
2. **Early authenticated chunks.** Additionally obtain the exact manifest
   commitment from an independent trusted channel or a separately specified,
   domain-separated quorum attestation binding codec version, genesis, height,
   block ID, state root, manifest and limits. Existing precommits must never be
   reinterpreted as signatures over that manifest. A new attestation requires
   explicit approval, durable signing reservations in a distinct versioned
   namespace, equivocation rules and a security review. M15 implements none of it.

Both models still require the complete final root/invariant checks. Hashes cannot
authorize an index sidecar, replacement nonce table, weaker label validation,
financial operation or alternate state-root algorithm. Registration/nonce-count
equality must retain its current meaning; updates/deletions require a separate
transaction-semantics proposal.

## Compatibility and upgrade order

1. Approve numerical capacity and byte/work budgets, deadline/peer admission rules,
   concurrent import/fork memory limits, checkpoint size and recovery-time targets.
   The laboratory's 500k/80-MiB bounds are measured inputs, not approved limits.
   Account diversity and maximum labels must both be covered.
   A decoder limit alone does not stop execution from growing past that limit:
   define safe checkpoint-refusal/retention behavior and obtain separate consensus
   approval for any new transaction/state-growth admission rule. Never silently
   reject formerly valid registrations or treat a 500k decoder as unlimited state.
2. Specify and audit the final envelope, transport model above, canonical chunk
   layout, durable publication/recovery protocol and independent rollback storage.
   Use a new production domain. Do not reuse `YC-LAB-STATE2-*` files or silently
   reinterpret them. Keep v1's exact bytes and limits and reject unknown formats.
3. Ship inactive readers and operator tooling first. Negotiate explicit reader
   capability; test mixed-version archives, pruned nodes, bootstrap, partitions and
   epoch transitions. Collect real large signed-history evidence on the target
   hardware with bounded hostile inputs and simultaneous live consensus load.
4. Establish quorum/operator readiness before crossing the old recovery boundary.
   Existing execution can grow beyond 100k, but old v1 pruned readers cannot
   serialize/import such a checkpoint. They must not silently enter a failure
   loop, switch storage modes, discard their retained history or accept v2 through
   a v1 parser. An archive fallback requires a complete authenticated prefix and
   an explicitly managed operator migration.
5. Activate a production writer only after explicit approval and review of the
   complete compatibility/security campaign. Keep automatic pruning disabled for
   v2 until approved finality/ancestry verification, full readback and independent
   rollback protection have all succeeded. Production activation is separate from
   enabling a build feature or running a lab example.

## Publication, recovery and downgrade rules

Use immutable generations. Pin exact context before writing payloads; cap chunk,
manifest, total-entry/byte, directory and aggregate concurrent work. Authenticate
and validate records before indexing. Sync pending files, rename and sync their
directories; publish the completion witness only after full destination readback.
Recovery reconstructs indexes and the flat root from authenticated records, never
from trusted-looking cache files. Define retry, partial-prefix and ENOSPC behavior
without overwriting conflicting bytes or deleting safety/history data.

Maintain the monotonic accepted checkpoint/rollback anchor independently of the
replaceable generation. Updating that anchor and installing a generation needs a
reviewed crash protocol that cannot roll back the independent minimum. A copied
old directory plus an old local anchor is insufficient rollback protection.
Preserve independently anchored signing/archive minima, identity fences, evidence,
WAL and every irrevocable reservation. A used validator identity must recover its
original safety state; importing an application snapshot never replaces it.

Old binaries must refuse an activated format they cannot verify. Downgrade must
never erase finalized blocks or rewind consensus to make an old snapshot fit.
Retain approved v1 checkpoints and full history according to their existing
policies; any later retention/pruning change requires its own explicit approval.

## Remaining gates

- Independent security and compatibility review of the complete implementation,
  not only the chunk codec; fuzzed canonical vectors and adversarial transport.
- Measured large live histories, committee transitions, large quorum/fault cases,
  retained branches, bounded network deadlines and resource admission under load.
- Recovery and synchronization service-level budgets, filesystem/power-loss
  testing, backup/restore drills and independently anchored rollback tests.
- An explicit decision on the flat root's linear per-height hashing/replay cost.
  A different authenticated-tree root would require a separate consensus version
  and a verified bridge; a snapshot upgrade cannot silently introduce one.

YPP, all financial proof versions, payments, staking and production elections stay
disabled throughout this proposal. No public deployment is authorized by it.
