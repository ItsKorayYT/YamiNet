# Relay integration verification

All results here are automated **loopback simulation**, not physical NAT or
cross-network tests. No real VPS/home/mobile-network trial has been performed.
See [public deployment and Windows hosting](public-relay.md) for that procedure.

Run from the repository root with the pinned Rust toolchain:

```sh
cargo test -p yami-browser --test relay_milestone --locked -- --nocapture
cargo test -p yami-relay-cli --test processes --locked
cargo test -p yami-relay --locked
cargo test -p yami-dht --locked
cargo test -p yami-node --test maintenance --locked
```

`four_process_outbound_host_dht_browser_render_navigation_and_reconnect`
launches four independent OS processes from the Rust test executable. They use
the production components rather than mock sockets or a separate network stack:

| Process | Production path |
| --- | --- |
| A: website host | Persistent identity, `Hosts`, `Registration`, `Maintainer`, `Worker`, `Site`, YNP dispatcher; **no listener** |
| R: relay | Explicit `Relay::bind_public` listener on loopback, real TLS/challenge/tunnel protocol |
| B: DHT peer | `TlsListener`, `Service`, signed records and real FIND/STORE dispatcher |
| C: browser | `Resolver`, bounded `Loader`, `Browser`, YLang/YSL parsers, `NativeDocument` layout and software drawing |

C starts with only B's manually pinned contact. It resolves A's full ID, checks
relay fallback is opt-in, authenticates R and A, renders a homepage to pixels,
follows its actual inert YLang yami:// link, goes Back and reloads. Assertions
check the unchanged canonical identity address and separately verified keys.
The test stops R, observes a newer withdrawal on B, restarts R at the same
endpoint, observes a newer route, and repeats browsing. It then stops A and
checks a stale advertisement cannot fetch a page. Temporary identity/state and
render artifacts are cleaned on completion. Every fixture has bounded lifetime;
process guards kill/reap children on failure.

| Failure or invariant | Automated coverage |
| --- | --- |
| Exact destination pin, including dishonest registered host serving another TLS identity | Existing relay CLI `three_processes_fetch_reconnect_and_close_when_host_disconnects`, native browser identity tests |
| Incorrect relay pin and failed direct endpoint followed by successful relay | `browser_direct_failure_relay_fallback_and_relay_pin_rejection` |
| Relay outer handshake timeout | `relay_outer_handshake_obeys_the_whole_attempt_deadline` |
| Forged/expired routed records; endpoint policy; every-byte tampering/truncation; rollback from v2 to v1 | DHT `relay_v2_binds_owner_relay_endpoint_and_preserves_v1` |
| Maximal v2 records in unchanged frame/snapshot bounds | DHT `maximal_relay_records_fit_existing_frame_and_snapshot_budgets` |
| Persistent sequences, relay-only withdrawal/renewal/restart | Maintenance `relay_only_publication_withdrawal_renewal_and_restart_keep_high_water` and four-process test |
| Temporary relay outage, reconnect/DHT update, disconnected host | Four-process test and existing three-process relay test |
| Registration replay, hijacking, bad expiry, channel substitution | Relay `fresh_challenges_reject_replay_hijacking_expiration_and_channel_substitution` |
| Competing clients, busy destination, global tunnel/per-IP limits | Relay `tunnels_and_per_host_and_per_ip_connections_are_bounded` |
| Malformed protocol versions, lengths, kinds and control requests | Relay wire/server tests and DHT wire tests |
| Bounded forwarding, bandwidth/byte quota cleanup | Relay `opaque_forwarding_closes_at_the_byte_quota_without_unbounded_buffering` |
| Off-thread loading, tabs, history, reload, direct ynps:// | Existing browser model, native DHT and desktop tests |

Full acceptance checks:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

Ignored physical-network, multicast-interface and screenshot tests remain opt-in.
A passing default suite does not establish their success, Windows runtime
compatibility, resistance to distributed DoS, or production security.
