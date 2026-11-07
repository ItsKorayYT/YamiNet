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

**Registration here does not add the candidate to Malachite's live committee.**
The financial ledger implements key-possession checks, owner-authorized exit,
self-stake thresholds, delegation ownership, supply-conserving locks, delayed
withdrawals and single-use tickets. Eligibility is derived from locked accounts,
not advertised peer weights. Live stake-driven selection/rotation, dynamic
consensus membership and financial slashing remain unfinished. The separate
objective double-vote verifier verifies real consensus evidence but applies no
penalty. Do not describe either component as completed permissionless PoS.

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
