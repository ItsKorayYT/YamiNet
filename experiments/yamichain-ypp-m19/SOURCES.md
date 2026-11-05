# Source record and review boundaries

Review date: 2026-11-05 (Europe/Berlin). This is a technical feasibility review,
not a paper audit, production security attestation or reproduction of upstream
benchmark results. Paper passages were read through the research tool; PDF byte
hashes are not claimed because the PDFs were not downloaded into the workspace.
The public MVP source was inspected locally at a pinned commit, without running
its clients, servers, key distribution, benchmarks or build scripts.

## Mandatory primary literature

1. Anrin Chakraborti and Radu Sion, **ConcurORAM: High-Throughput Stateless Parallel
   Multi-Client ORAM**, NDSS 2019.
   [Paper](https://www.ndss-symposium.org/wp-content/uploads/2019/02/ndss2019_07B-2_Chakraborti_paper.pdf),
   [publication page](https://www.ndss-symposium.org/ndss-paper/concuroram-high-throughput-stateless-parallel-multi-client-oram/).
   Relevant locations: section IV-A (printed pages 3–4, trust model, statelessness,
   map design); IV-B (query/result logs and coordination); section V (costs);
   section VII-B (malicious-server extension). No client-isolation theorem is
   inferred from a server-security statement.

2. Robin Vassantlal, Hasan Heydari, Bernardo Ferreira and Alysson Bessani,
   **MVP-ORAM: a Wait-free Concurrent ORAM for Confidential BFT Storage**, NDSS 2026.
   [Author-hosted conference PDF](https://www.di.fc.ul.pt/~bessani/publications/ndss26-mvporam.pdf),
   [extended version, arXiv:2512.12006v1](https://arxiv.org/html/2512.12006v1),
   [artifact](https://github.com/rvassantlal/MVPORAM).
   Main review locations: section III (system/service/adversary and security
   definition), IV–V (memory operations and version handling), VI (replication
   and key management), VII-C/Table II (cost model), VIII (Strong variant).
   The extended version is explicitly pinned to v1, dated 12 December 2025;
   conference year and preprint date are not conflated.

3. Christian Cachin, David Lehnherr, Juan Villacis and François-Xavier Wicht,
   **The Consensus Number of Untraceable Cryptocurrencies**, AFT 2026,
   LIPIcs 395, 5:1–5:24; published 2 October 2026.
   [DOI/landing page](https://doi.org/10.4230/LIPIcs.AFT.2026.5),
   [published PDF](https://drops.dagstuhl.de/storage/00lipics/lipics-vol395-aft2026/LIPIcs.AFT.2026.5/LIPIcs.AFT.2026.5.pdf),
   [full-version HTML consulted for LUAT](https://arxiv.org/html/2607.20929v1).
   Published-paper locations: section 3 (object/consensus-number model),
   definition 4 (CUAT), theorem 14 (incidence assumptions), corollary 17
   (one-round bound), theorem 23 (general strong-variant consensus number),
   theorems 24–25 (workload graph), theorem 27 (starvation).
   Full-version section 5 covers LUAT; its shared-account case differs from
   unshared accounts. README quotes the unshared result only. The published
   theorem and the full-version result are identified separately.

## Publicly verifiable storage scope check

**Externally Verifiable Oblivious RAM**, PoPETs 2017,
[publication](https://petsymposium.org/popets/2017/popets-2017-0021.php),
[ePrint 2017/008](https://eprint.iacr.org/2017/008.pdf), section 3.
The trusted-arbitrator model is not adopted as public monetary verification.
This limited source check does not evaluate or reject every verifiable ORAM.

## Pinned source-level corroboration

Repository: https://github.com/rvassantlal/MVPORAM

Commit: `6d51057bf3ccd50c9374703586be1a88bb88b08f`.

All paths below are relative to that repository. Line numbers refer to this commit.
Hash records are in `source-files.sha256`; none of this source is copied or
introduced as a dependency of YamiChain.

| Source | Inspected behavior relevant to the rejection |
|---|---|
| `src/main/java/oram/client/manager/ORAMManager.java:32` | Creates the encryption password and initial encrypted metadata |
| `src/main/java/oram/client/manager/ORAMManager.java:52` | Obtains confidential ORAM material and initializes the client's encryption manager |
| `src/main/java/oram/client/manager/MultiServerORAMManager.java:28` | Sends confidential creation material through the confidential service |
| `src/main/java/oram/server/ORAMServer.java:44` | Stores received key shares and supplies them with the ORAM context response |
| `src/main/java/oram/security/EncryptionManager.java:40` | Decrypts labelled path-map histories |
| `src/main/java/oram/security/EncryptionManager.java:142` | Encrypts/decrypts stashes; bucket handling follows in the same file |
| `src/main/java/oram/client/ORAMObject.java:75` | Map consolidation, address translation, block operation, eviction |
| `src/main/java/oram/client/ORAMObject.java:193` | Version/access/location-update metadata merge and missing-version tracking |
| `src/main/java/oram/client/structure/PositionMap.java:8` | Four full-sized integer arrays and serialization formula |
| `src/main/java/oram/server/ORAM.java:140` | Stores eviction/map versions; `cleanPositionMaps` is empty in this snapshot |
| `src/main/java/oram/utils/ORAMContext.java:11` | Tree size and client address-space dimensions |
| `src/main/java/oram/utils/ORAMUtils.java:30` | Complete-tree node-count formula |

This is a bounded inspection of the key/metadata path. No correctness,
constant-time, cipher-security, key-erasure, artifact-remediation or deployment
audit is implied. In particular, research artifact code is not assumed to
perfectly instantiate every paper assumption or optimization.

## Earlier repository reports

The core rejection decisions and READMEs were read before ORAM assessment;
associated proof specifications and validation reports were consulted to preserve
scope and avoid treating primitive measurements as end-to-end results.
`prior-reports.sha256` inventories the earlier YPP Markdown reports, the historical
privacy gates and the RingCT reference. The only YPP research directories in the
base tree are vaults, hidden, feasibility, oblivious, accounts and m18; their
milestone numbering must not be confused with unrelated chain-storage milestones.

No summary above reclassifies a previous rejected construction as accepted.
