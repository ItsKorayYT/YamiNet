# YNP game resources: wire version 3

Documents retain wire v1, PNG page assets retain v2, and game resources use v3
inside the existing `ynp/1` TLS channel. There is no game plaintext mode, automatic
fallback, redirect, generic URL fetch or new relay protocol. Each resource uses
one authenticated connection and one request. The publisher proves the exact node
ID inherited from the parent page. A relayed connection also pins the parent's
exact relay ID and performs independent inner TLS to the publisher. Existing DHT
signed records locate nodes; endpoint claims never replace TLS authentication.
Clients are readers; this does not add client authentication or publisher accounts.

The unchanged 16-byte header contains `YNP\0`, version=3, kind, two zero reserved
bytes, u32 payload length and u32 request ID. All integers are big endian.

| Kind | Payload |
| --- | --- |
| 1 GetGame | u16 key length, key, u8 resource type |
| 2 Game | u16 key length, key, u8 resource type, u32 byte count, bytes |
| 3 GameNotFound | u16 key length, key, u8 resource type |

Resource type 1 is a UTF-8 JSON manifest (1..4096 bytes). Type 2 is binary Wasm
(1..1,048,576 bytes). Keys follow v2's safe 1..64 ASCII key rules, including its
Windows device-name prohibition. Paths, separators, extensions and identities
are invalid. Codecs reject unsupported types, sizes, reserved bits, truncation
and trailing payload bytes; the client also verifies version, request ID, key
and requested type. The decoder reads the bounded key/type prefix before any
resource allocation and enforces type-specific bounds. Legacy nodes may close
v3 requests; that becomes a native game failure with an explicit Restart action.

## Hosted manifest 1

This differs deliberately from the existing local path-based manifest:

```json
{
  "version": 1,
  "abi": 1,
  "title": "Prism Breakout",
  "description": "Clear the prism wall.",
  "module": {
    "key": "prism",
    "bytes": 24138,
    "sha256": "THE_64_LOWERCASE_HEX_DIGITS_OF_THE_MODULE"
  }
}
```

The example digest and length must be replaced by the publisher's actual values;
the `publish` example generates them. Only version 1 and ABI 1 are accepted.
Unknown and duplicate fields fail. Titles are 1..80 UTF-8 bytes, descriptions
0..512, neither permits controls. The module length is 1..1 MiB, its key is a
safe resource key, and its digest is exactly 64 lowercase hexadecimal digits.
The client checks exact length, SHA-256 and the Wasm v1 binary header before
constructing the existing VM. Wasmi then validates the module, imports, exports
and ABI. The digest authenticates consistency with the manifest received from
the pinned publisher; it is not an endorsement or an independent code signature.
The existing `site.sig` still covers only pages and style.

ABI 1 supports drawing commands and text/constants embedded in the Wasm module.
Those game resources travel with the module. No external guest resource API
exists in this runtime, so external resource lists, filesystem paths and network
capabilities are rejected as unknown fields. Website PNGs remain supported by
the existing bounded v2 loader; they do not grant guests image or network access.

## Publication and activation

Nodes serve only `games/<key>.json` and `games/<key>.wasm` under a directory site.
Both the games directory and files must be regular non-symlink entries with exact
canonical parent/name checks and bounded reads. Publishers control these local
files. This is not protection against hostile concurrent local filesystem changes
or hard links; keep site directories private. Site check/build verifies references
and module integrity, copies only validated manifests/referenced modules, and
never executes a game. The publisher-side preparation example validates a local
game with the VM before writing a new publication; players never run that tool.

YLang 0.2 adds an inert link: `> game:prism | Prism Breakout`. The parser supplies
the fixed visible prefix **Play game:**. It does not fetch or launch anything.
Only the native pointer/keyboard activation handler calls `Browser::play_hosted`.
That method requires an offered link on the current ready, TLS-authenticated page
and an origin whose destination matches that page's observed security identity.
Address entry, normal link navigation, history restoration, bookmarks, Discover,
page parsing, images and page reload never call the game loader. Plaintext and
built-in pages cannot deliver hosted executable content.

Play opens a separate ordinary game tab with the authenticated publisher/security
state after successful loading, keeping the source page and existing tabs.
Loading and failed pre-execution transfers do not display a verified-delivery
badge; a validated first frame establishes that the authenticated load succeeded.
Back/forward return to inert
browser locations. Stop drops the VM. Restart after stop/failure re-fetches from
the same publisher route under fresh limits. Reset of a running VM uses the
existing fuel-bounded reset. There is no installation, executable file cache,
external launcher or persistent game byte storage in the browser.

## Bounds and shutdown

The existing four game-worker admission slots include loading, compilation and
running sessions; no network I/O, Wasm validation or compilation runs on the UI
thread. One load has an eight-second absolute deadline across both resources,
including TCP, TLS, relay pairing, waits and partial I/O. Its application-read
budget is 4096 + 1,048,576 + 2×87 = **1,052,846 bytes**, including YNP headers.
TLS/control overhead uses the existing transport/relay limits. There are two
resource requests maximum. Direct connections are not retried. A refused relay
pairing permits one jittered 400..500 ms reconnect per resource to accommodate
the existing host's single-use registration renewal. Identity failures and resource
transfer/validation failures never retry. Both attempts retain the same pins and
deadline; no route or identity substitution occurs.

Stop, tab navigation/close and session drop signal cancellation, prevent subsequent
requests and VM creation, and never join a transfer on the UI thread. In-flight
socket work finishes within the remaining eight-second transport deadline; it
is not necessarily interrupted immediately. Workers release admission and socket
state after returning. Old workers publish only into their own session mailbox,
so they cannot mutate a replacement tab or navigation generation.

The existing ABI sandbox is unchanged: four drawing imports only; no WASI,
filesystem, networking, process or OS calls; 4 MiB linear memory, one memory,
one table of at most 1024 elements, recursion depth 128, stack height 64 Ki,
200,000 fuel per start/ABI/init batch and per reset/update, 512 commands per frame,
and bounded drawing area/text. Pausing releases input and suspends execution.
Compilation uses the existing eager Wasmi strict limits and 1 MiB input bound;
it is not governed by a hard wall-clock kill or a separate OS process limit.
These are experimental defenses, not production-grade sandbox security.
