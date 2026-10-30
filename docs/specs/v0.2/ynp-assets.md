# YNP asset extension: explicit wire version 2

Original document frames remain byte-for-byte wire version 1 and use the original
Get/Document/NotFound payloads. Assets use the same 16-byte `YNP\0` frame header,
request correlation and authenticated transport, with version byte **2**:

| Kind | Asset payload (big-endian lengths/integers) |
| --- | --- |
| 1 GetAsset | u16 key length, key bytes |
| 2 Asset | u16 key length, key, u8 format=1 (PNG), u32 byte count, PNG bytes |
| 3 AssetNotFound | u16 key length, key |

Reserved header bytes must remain zero. Keys are 1..64 ASCII letters, digits,
underscores or hyphens, with Windows device names forbidden. No separators,
dots, filenames, URLs or identities are accepted. Asset bytes are 1..262144.
The decoder checks kind/length/budget before allocation and rejects extra payload
bytes, unknown formats, wrong response keys or mismatched request IDs.

The existing `ynp/1` TLS channel carries these explicitly versioned messages;
there is no plaintext public asset transport. The node accepts one request per
connection through the existing dispatcher. Both client and server require the
parent's pinned TLS 1.3 policy for assets. For relayed pages each asset uses the
same selected relay pin/endpoint and independent exact destination pin. The
relay continues forwarding opaque inner TLS, with its existing limits.

A legacy node may reject v2 and close the connection. The browser displays the
image's alternative text; it does not retry v1 as plaintext or interpret document
responses as images. Existing v1 websites and messages are unchanged.

Directory sites serve only `assets/<key>.png`. The assets directory and file
must be regular non-symlink entries under the exact canonical site root/name;
traversal, aliases, devices and non-PNG signatures are refused. Site writers are
trusted locally: canonical checks do not sandbox hostile concurrent filesystem
changes or hard links. Keep site directories private to their owner.

PNG decoding uses the maintained Rust png library, ignores text/ICC metadata,
accepts static PNG only, validates checksums/end chunks, and enforces 2048 per
dimension, 1,048,576 pixels and 4 MiB prepared RGBA per image. Decoder internal
allocation is separately capped at 8 MiB. No metadata is executed or rendered.

The browser loads at most eight distinct same-owner images, sequentially within
each of its existing two loader workers. Each page permits 1 MiB incoming asset
frame/PNG data and 16 MiB retained decoded pixels. Failed and malformed responses
consume transfer budget too. At most two connection attempts per key share a
three-second budget and the original queued navigation deadline. Cancellation
stops new requests; in-flight socket work ends within its bounded deadline.
Stale completions cannot alter a different page/tab/generation.

Documents become ready before assets complete. PNG decoding and premultiplication
run off the UI thread. Images preserve aspect ratio, fit YSL maximum width/height,
participate in document flow and scroll/clip with the page. Pending, missing,
corrupt, over-budget or unsupported images display meaningful alternative text.
There are no cross-identity image URLs, external font/resource fetches, executable
metadata, persistent asset cache, animation or generic media system.
