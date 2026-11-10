# Private fee and staking-accounting devnet

This is an **insecure disposable-YAMI development network**, not production PoS.
It reuses the existing four executors, external authenticated witnesses and
continuous native-certificate Malachite validators. The previous devnet and
ledger are retained separately.

```sh
./scripts/yamichain-economic-devnet start
./scripts/yami-wallet create work/alice.secure
./scripts/yami-wallet create work/bob.secure
BOB=$(./scripts/yami-wallet address work/bob.secure)
./scripts/yamichain-economic-devnet faucet work/alice.secure 1000
./scripts/yami-wallet fee Transfer
./scripts/yami-wallet send work/alice.secure "$BOB" 100
./scripts/yami-wallet balance work/alice.secure # 898: 100 sent + 2 fee
./scripts/yami-wallet balance work/bob.secure   # 100; recipient was offline
./scripts/yami-wallet verify 2                 # use the returned actual height
./scripts/yami-wallet backup work/alice.secure /secure/offline/alice.backup
./scripts/yami-wallet restore /secure/offline/alice.backup work/alice-restored.secure
```

The encrypted keystore uses scrypt (N=131072, r=8, p=1; fixed bounded memory) and
ChaCha20-Poly1305. Its address/version are authenticated. Passwords are prompted
locally; automation can use `YAMI_WALLET_PASSWORD_FILE` pointing to a mode-0600
file. Passwords/seeds never enter process arguments or network requests. Keep the
encrypted backup and password separately. Password loss is not recoverable.
Use `yami-wallet encrypt OLD_WALLET NEW_ENCRYPTED_WALLET` to migrate a legacy key.
The original file is retained; protect any remaining plaintext copy.
Legacy plaintext devnet wallets are readable for compatibility, but the new
backup command refuses to copy plaintext keys. Wallet OS security remains trusted.

Candidate registration and locked staking/delegation accounting:

```sh
VALIDATOR=$(./scripts/yami-wallet validator-key work/operator.validator)
./scripts/yami-wallet register work/alice.secure work/operator.validator
./scripts/yami-wallet stake work/alice.secure "$VALIDATOR" 200
./scripts/yami-wallet unstake work/alice.secure "$VALIDATOR" 100
./scripts/yami-wallet balance work/alice.secure
# After the reported withdrawal ticket's unlock height is finalized:
./scripts/yami-wallet withdraw work/alice.secure TICKET_ID
./scripts/yami-wallet exit work/alice.secure "$VALIDATOR"
```

**Registration in this fixed economics profile does not add the candidate to Malachite's live committee.**
The separate [live PoS profile](pos.md) integrates stake selection, rotating
committees, rewards and evidence-based slashing into the running network.
The financial ledger implements key-possession checks, owner-authorized exit,
self-stake thresholds, delegation ownership, supply-conserving locks, delayed
withdrawals and single-use tickets. Eligibility is derived from locked accounts,
not advertised peer weights. Live stake-driven selection/rotation, dynamic membership and slashing stay
disabled in this fixed profile. The live profile applies the existing objective
evidence verifier inside encrypted execution.

The sample [configuration](economics-network.json) explicitly sets the example
fee schedule, resource-unit limits, stake threshold, unbonding/epoch parameters,
active-set bound and fee recipients. These are disposable-network inputs, not
permanent monetary policy. The actual consensus committee is currently the four
bootstrap validators; fees go to their configured reward accounts, with equal
shares and deterministic rotating remainders. Delegator reward sharing and
stake-driven reward recipients are not activated. No tokens are issued.

Fees are charged within the encrypted signed operation; the wallet authorizes a
maximum, and execution charges the policy's exact required fee. A transfer uses
one resource unit, registration four, bond/unbond two and exit/withdrawal one.
Account balances + bonds + pending withdrawals always equal the genesis supply.
All four executors independently check this before releasing witnessed votes.
Certificates expose neither transfer fields nor private portfolio balances.

Policies are pinned at a fresh genesis. Existing fee-free states retain their
original canonical encoding and can still replay. Changing an economic policy
requires a new development network/data directory; it never silently rewrites
old balances. Configure `chain` with a distinct numeric chain ID for a separate
network; existing deployments default to their original ID 24. An unbonding
period counts finalized private-state transitions, not elapsed wall-clock time.
Keep liquid YAMI for fees when requesting unbonding. A mature withdrawal can pay
its fee from released proceeds.

Endpoint: `http://127.0.0.1:29780/status`, local to the running environment.
Data: `work/yamichain-economics-devnet`. `YAMI_DEVNET_CONFIG` and
`YAMI_DEVNET_DIR` select multi-machine configurations using the existing endpoint
schema. `yami-wallet --network native` selects the original development profile.
TDX wallets continue using `yamichain-tdx-wallet` and genuine wallet-local QVL;
fee authorization is supported there without a software fallback. Rebuild guest
images and approve their new measurements before running this changed execution
code on TDX; the previous eddfbc3 image does not contain this accounting code. Hardware
execution has not been exercised, and recoverable executor-custodian secrets
are still a security dependency, not production-secure custody.

```sh
./scripts/yamichain-economics-smoke # fresh dedicated economics ledger only
./scripts/yamichain-economic-devnet stop
./scripts/yamichain-economic-devnet start
```

The executed smoke includes native finality, private fees, registration,
bond/unbond, early/double-withdrawal rejection, exact reward shares and restart.
The encrypted-wallet check separately exercises payment, backup/restore and
local verification of the full finalized native history. Records are beside
this file. Production financial proofs and activation gates remain closed.

The authenticated private portfolio includes the caller's individual `delegations`
(validator, owner, amount), withdrawal tickets and current fee-recipient status.
Reward terms explicitly report that this disposable policy pays all fees to the
active validator owners (or configured recipients in the fixed profile), with
no direct delegator rewards. The reported 10000 basis points describes this
existing allocation, not a configurable commission or a promise of future yield.
These additions are encrypted query data; financial state encoding is unchanged.

Outgoing transaction history is wallet-local and encrypted with a separate,
network-bound key derived from the unlocked wallet seed:

```sh
./scripts/yami-wallet transactions work/alice.secure
./scripts/yami-wallet retry work/alice.secure TRANSACTION_ID
```

`transactions WALLET [OFFSET [LIMIT]]` lists newest submissions first, defaults
to 20 entries and accepts a maximum of 100; `next_offset` continues the listing.
Each status still verifies the required native history prefix; large-history
checkpoint verification remains a separate unfinished requirement.

Every submission is durably recorded before transmission, including staking
operations. `send` returns the blinded transaction ID. `transactions` verifies
native finality and reports `finalized`, `superseded` (another authentic transition
used that parent), or `unknown` (missing, unavailable or unverifiable finality).
An RPC timeout remains uncertain. `retry` reuses the exact original payment;
it never creates a fresh payment or nonce. A changed parent requires finality
verification before reporting a result. There is no automatic repeat payment.

This history covers outgoing submissions from this wallet installation, not
incoming payments or independently recovered historical activity. The encrypted
`.history-<network>.json` sidecar is separate from the keystore backup; copy it
separately for history recovery. Keep the sidecar mode 0600. Files are bounded
at 1024 entries and 2 MiB plaintext; archive a full encrypted history before
submitting more. Preserve archived files under separate names. Removing history
loses local information about uncertain submissions; it does not reverse payments.
The OS remains trusted for availability and rollback of local history. History
contents grant no authority and are never accepted as a finality certificate.

Economic version rules: [policy versions](economic-policy-versions.md).

A wallet may configure `wallet_gateway_seeds` as 1–8 unique HTTPS service origins
in its trusted `YAMI_DEVNET_CONFIG`. On each command it selects the first seed
that passes CA/hostname verification, role authentication and exact policy-pin
comparison. Supply `wallet_policy_file` with an independently obtained copy of
the network policy when the wallet data directory has no pin yet. Multiple seeds
refuse to run without that pin. Each seed probe has a three-second timeout;
redirects remain refused. A foreign network, invalid policy or failed probe
cannot replace the pin. No endpoint can supply a new CA or credentials.
Discovery chooses a gateway before submission; it does not automatically repeat
an uncertain payment. Existing single-endpoint first-use pinning is retained.
These are configured bootstrap seeds; public permissionless P2P discovery and
cross-operator deployment validation remain unfinished.
