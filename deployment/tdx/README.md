# TDX executor deployment

**Not deployed to TDX in this environment. No hardware-attested payment has been
claimed. Production/real-money activation remains CLOSED.** The environment has
no TDX guest device, KVM, cloud credentials, outbound cloud identity or gcloud.
The package below is implemented and built; hardware boot, fresh quote generation,
live Intel PCS retrieval and the hardware Alice-to-Bob demonstration need the
external resources specified below. The existing software devnet stays separate.

## Build the immutable guest

Build dependencies: Linux x86-64, Rust 1.90, Python 3, C/C++ compiler, make,
objcopy, dpkg-deb, OpenSSL development libraries. No Docker or privileged mount
is needed. Debian guest inputs have exact versions and SHA256 pins in
`inputs.json`; kernel config checks require Intel TDX, its guest driver and EFI.
The Intel quote-generation source is also fixed to an official commit.

```sh
cargo build --release --locked \
 --manifest-path experiments/yamichain-tee-m20/Cargo.toml \
 --features tdx-guest-devnet \
 --bin tdx-executor --bin tdx-admin --bin attested-wallet --bin devnet-bridge
./deployment/tdx/build-intel.sh work/tdx-intel
./deployment/tdx/fetch-inputs.py work/tdx-inputs
```

On **each executor's separate custodian machine**, generate a distinct key file
and public image configuration (replace `0` per member). `4050` is an example
QGS vsock port; use the port supported by the selected TDX provider:

```sh
./deployment/tdx/manage.py custodian-key work/custodian-0.json
./deployment/tdx/manage.py guest-config --index 0 \
 --key work/custodian-0.json --qgs-port 4050 --out work/guest-0.json
./deployment/tdx/build-guest.py --inputs work/tdx-inputs/root \
 --executor experiments/yamichain-tee-m20/target/release/tdx-executor \
 --quote-library work/tdx-intel/QuoteGeneration/quote_wrapper/tdx_attest/linux/libtdx_attest.so \
 --public-config work/guest-0.json --out work/tdx-guest-0
./deployment/tdx/validate-package.py work/tdx-guest-0
```

Outputs: an immutable `initrd.cpio`, `BOOTX64.EFI` unified kernel image (UKI),
sparse GPT/EFI `disk.raw`, and `manifest.json` containing hashes of every regular
guest file, kernel, initramfs, UKI and disk. The packaging is byte-reproducible
for identical input artifacts; two complete builds were compared. Rust/native
compiler outputs and their linked runtime libraries are explicit hashed image
inputs. This is not a claim that arbitrary host toolchains produce identical
binaries. Rebuild with independently reviewed toolchains for release approval.

The whole executor, runtime libraries, public configuration and modules are in
the UKI's initramfs. There is no mutable host root filesystem, cloud-init, SSH,
remote shell, imported private executor key or externally loaded application.
Guest kernel RNG creates distinct Ed25519 signing, X25519 transport and storage
keys in guest RAM. Quote generation uses the pinned real Intel `tdx_att_get_quote`
ABI; missing TDX support fails startup. No simulated quote backend exists.

## Cloud resources and independent placement

Needed externally: authenticated cloud projects with Intel **TDX** capacity and
quota, a supported machine type/zone, custom UEFI image import permission,
storage buckets, private subnets with routing to gateway and witness services,
and a functioning provider QGS/configfs quote path. Cloud API credentials must be
configured in the deployment environment, not pasted in chat. The supplied
scripts target Google Compute Engine's TDX launch option; no cloud API operation
was possible here and these launch commands have not been exercised.

Provision four pinned executor slots; at least three must be reachable and pass
native wallet QVL verification. Use at least three distinct physical-host
placements and independently administered executor projects/operators. Prefer
separate supported zones or a provider-enforced physical separation policy.
Four VMs on one host, different VM names or separate directories do not prove
independence. Cloud metadata is not accepted as TDX evidence. The common cloud,
Intel hardware/firmware and administrative dependencies remain in the TCB.

For each executor image and its assigned project:

```sh
./deployment/tdx/import-gcp.sh PROJECT BUCKET yami-tdx-0 work/tdx-guest-0
./deployment/tdx/launch-gcp.sh PROJECT ZONE PRIVATE_SUBNET yami-executor-0 \
 yami-tdx-0 TDX_MACHINE_TYPE LEAST_PRIVILEGE_SERVICE_ACCOUNT
```

Choose a provider-supported Intel TDX machine/zone (for example an eligible C3
configuration after checking that project's current capacity). The scripts use
no public IP and no VM cloud API scopes. The supplied UKI is unsigned, so this
profile disables Secure Boot; wallet policy must still pin the exact approved
TDX boot measurements. Signing/enrolling the UKI and enabling Secure Boot is a
separate boot profile requiring new approved measurements. This never disables
QVL, revocation, TCB, debug or production-payment checks.

Deploy four witness authorities with distinct keys under **separate witness
administration and storage outside each executor's rollback domain**. Do not
install the corresponding witness on its executor host or give its executor
administrator access to its witness signing key/SQLite database. Persist witness
SQLite files and WALs. Separate private networking/processes alone do not provide
independence. Allow only gateway/custodian/wallet clients to reach guest TCP
28790 and executors to reach their pinned witness TCP authorities.

## Pin measurements, provision and connect the native network

Copy/edit `network.example.json`, replacing example private IPs with actual
addresses. Validators/gateway may share a devnet host; executor and witness
independence is a separate requirement. Use new data/ports so the original
`work/yamichain-native-devnet` is preserved. The resulting gateway URL is the
`gateway` value in this configuration; **the example IP is not a running endpoint**.

Each witness operator generates its own witness key using the existing bridge:

```sh
printf '{"op":"key"}' | experiments/yamichain-tee-m20/target/release/devnet-bridge \
 > /secure/witness-0.key
chmod 600 /secure/witness-0.key
```

Exchange only the four public `address` values, converted to 32-byte JSON arrays,
and place them in a four-element `authorities.json` in pinned index order. After
TDX guests boot and `/identity` is reachable, construct the provisional committee:

```sh
./deployment/tdx/manage.py plan --network /srv/yami/network.json \
 --authorities /srv/yami/authorities.json --data /srv/yami/tdx-network
```

This command **does not verify or approve hardware**. It fixes a new disposable
native genesis, finite treasury and public committee from provisional guest
identities. It never migrates/reset the existing ledger. Each custodian's next
step verifies its own guest with Intel QVL before encrypting provisioning secrets.
The trusted genesis coordinator temporarily knows the shared root-blinding
secret; this confidentiality dependency must be accepted for this disposable
bootstrap, and the private provisioning plan must be protected/deleted after
it is no longer needed. It never contains wallet signing keys other than the
separate locally held disposable treasury wallet.

Independently approve each guest's **MRTD and all four RTMR values**, boot
attributes, owner/config fields and optional TDX 1.5 extension using the provider's
firmware/measurement reference and the exact UKI. File SHA256 values are **not**
MRTD/RTMR values. `software-policy.template.json` deliberately has invalid empty
measurements; it cannot pass verification. Set its `identity` to that image's
manifest `software_identity`. Never auto-approve measurements read from an
unverified remote quote. Genuine approved boot measurements are unavailable in
this hardware-free environment and are a required external deployment input.

Build the existing official wallet QVL with
`experiments/yamichain-tee-m20/native/build_and_test.py`; use its recorded library
path/digest. Assemble the wallet's trusted configuration locally:

```sh
./deployment/tdx/wallet-config.py --qvl-library /trusted/libyami_intel_qvl.so \
 --qvl-sha256 APPROVED_QVL_SHA256 \
 --witness-policy /srv/yami/tdx-network/policy.json \
 --software-policy approved-0.json approved-1.json approved-2.json approved-3.json \
 --out /secure/tdx-wallet.json
```

Provision each witness from its own key and public policy, then run it on its
separate witness host (copy the binary and config with restricted permissions):

```sh
./deployment/tdx/manage.py witness-config --index 0 \
 --key /secure/witness-0.key --policy /srv/yami/tdx-network/policy.json \
 --listen WITNESS_PRIVATE_IP:28800 --database /srv/yami/witness-0.sqlite \
 --out /secure/witness-0.json
experiments/yamichain-tee-m20/target/release/witness-node /secure/witness-0.json
```

Build `witness-node` with the existing Cargo manifest if it is not already built.
Each custodian then runs its own provisioning command, with the public policy
and initial provisioning plan securely supplied by the genesis coordinator:

```sh
./deployment/tdx/manage.py provision --index 0 \
 --network /srv/yami/network.json --data /srv/yami/tdx-network \
 --key /secure/custodian-0.json --trusted-config /secure/tdx-wallet.json \
 --backup /secure/executor-0.recovery
```

The fresh quote binds keys, boot nonce, software identity and custodian keys.
Native wallet/custodian QVL verifies real Intel signatures, collateral expiry,
revocation, TCB/debug policy and approved measurements. Provisioning is encrypted
to the verified guest key and signed for its boot nonce. The guest checks its
committee slot, authenticated witness and genesis commitment. The encrypted
recovery capsule is fsynced outside the VM **before** signed backup confirmation
activates voting. A lost configure response can retrieve the same encrypted
capsule. It never resets an initialized witness.

`intel-pcs` in the deployment configuration retrieves fresh collateral using
Intel's HTTPS PCS APIs. Allow outbound access to `api.trustedservices.intel.com`
and any PCS certificate redirects from the custodian/wallet environment, or
supply an internal collateral URL in the existing JSON format. Collateral is
untrusted input and cannot authorize anything without the pinned native QVL.
Live PCS retrieval has not been exercised here because no cloud quote exists.

Build/start continuous validators and gateway on their private host:

```sh
cargo build --release -p yami-validator --features insecure-devnet-input
export YAMI_DEVNET_CONFIG=/srv/yami/network.json
export YAMI_DEVNET_DIR=/srv/yami/tdx-network
./deployment/tdx/run-network.py
```

This launcher runs **only** the existing native validators/gateway, using remote
executor endpoints. It never substitutes software executors. Existing full native
certificates, 3-of-4 financial signatures, witness confirmations, continuity and
Malachite finality checks remain unchanged. Network services remain an insecure
disposable devnet: public consensus fixture keys, unauthenticated HTTP metadata
and no real-money safety claim. The gateway's existing insecure warning remains;
only the local wallet's real QVL decision can authenticate private execution.

## Hardware payment demonstration command

After the above external deployment succeeds:

```sh
./scripts/yamichain-devnet wallet work/tdx-alice.wallet
BOB=$(./scripts/yamichain-devnet wallet work/tdx-bob.wallet)
./scripts/yamichain-tdx-wallet --trusted-config /secure/tdx-wallet.json \
 --deployment /srv/yami/network.json faucet work/tdx-alice.wallet 1000
./scripts/yamichain-tdx-wallet --trusted-config /secure/tdx-wallet.json \
 --deployment /srv/yami/network.json send work/tdx-alice.wallet "$BOB" 100
./scripts/yamichain-tdx-wallet --trusted-config /secure/tdx-wallet.json \
 --deployment /srv/yami/network.json balance work/tdx-alice.wallet # 900
./scripts/yamichain-tdx-wallet --trusted-config /secure/tdx-wallet.json \
 --deployment /srv/yami/network.json balance work/tdx-bob.wallet   # 100, offline receiving
```

Each payment and balance query uses fresh wallet challenges. Encryption occurs
inside `attested-wallet` only after at least three genuine native QVL decisions;
missing/bad/expired evidence gets no ciphertext, and fewer than three aborts
without submission. There is no software fallback. The wallet also checks full
native finalized history and the requested payment's transaction commitment.
Attestation never replaces the financial quorum. **These hardware transfer
commands have not been executed successfully in this environment.**

## Recovery and remaining trust

Guest keys stay in RAM; the encrypted recovery capsule and witness record live
outside its rollback domain. On guest reboot, keep the original policy, gateway
history, witness database and custodian keys/capsule. Run the same `provision`
command with the existing backup path: fresh enrollment is verified, old seeds
are re-encrypted to the new boot's ephemeral key, its pinned identity is restored,
and state loads from its authenticated witness. Wallet live quotes then bind the
restored committee identity. Do not run `plan` again on an existing network.
Loss of the custodian key or capsule can make that executor unrecoverable.

**The recovery custodian can recover executor keys, decrypt its private ledger
and impersonate that executor.** Separate custodians are therefore part of the
independent-trust assumption. Three compromised executors/custodians can falsify
private financial state; witness compromise permits rollback/equivocation for
its paired member and can invalidate quorum-intersection safety in sufficient
combination. TDX also trusts approved firmware/kernel/runtime and Intel's
certification infrastructure. No zero-trust custody or real-money financial
activation is demonstrated. A different attested key broker/sealing design would
require separate review; it is not claimed as implemented here.

`validation.json` records what was actually exercised: deterministic packaging,
PE/GPT/FAT validation, guest negative/security tests, real native QVL tests with
authentic archived Intel fixtures, strict Clippy and production proof rejection.
No software transfer is presented as a hardware demonstration.


Reproduce the non-hardware validation with the trusted QVL source build available:

```sh
export YAMI_QVL_LIBRARY=/trusted/libyami_intel_qvl.so
export YAMI_QVL_SHA256=APPROVED_QVL_SHA256
export YAMI_INTEL_DCAP=/path/to/pinned/intel-dcap
export YAMI_TDX_GUEST_BUILD=work/tdx-guest-0
./deployment/tdx/test.sh
```

The current managed environment also needs cloud API and Intel PCS network
allowlist entries before remote deployment: only package/source destinations are
presently configured. Do not interpret environment readiness or a successful
image build as evidence that a TDX VM is running.
