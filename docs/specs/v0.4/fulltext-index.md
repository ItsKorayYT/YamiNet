# Search publication format 1 (normative)

Search publications are inert YLang 0.2 documents served as ordinary YNP
documents over existing identity-pinned TLS. They introduce no transport,
signature algorithm or query service. The authenticated publisher asserts the
data; referenced website identities and text remain untrusted claims.

## Manifest

The stable key is `search-manifest`. Exact metadata is `title Yami Search
Manifest`, without language metadata. There is exactly one top-level section
`:: search-manifest 1`. Its children, in order, are childless sections:

```text
  :: pages N
  :: bytes N
  :: shard search-0000-HASH DOCUMENT_BYTES HASH
  :: shard search-0001-HASH DOCUMENT_BYTES HASH
```

`pages` is the decoded record count (0..256); `bytes` is the concatenated
decoded UTF-8 JSON payload size (1..6,291,456). There are 1..64 shard references,
exactly ceil(bytes/98,304). Decimal integers must be canonical (no signs or
leading zeros). Indices start at 0000 without gaps. HASH is lowercase SHA-256
hex of the entire shard source, including newlines, using the existing identity
crate's SHA-256. DOCUMENT_BYTES must equal the source byte length and must not
exceed 262,144. References are keys, never URLs or filesystem paths. Duplicate,
out-of-order, unknown, inconsistent or surplus nodes are invalid.

## Shards

Exact metadata is `title Yami Search Shard`, without language metadata. One
top-level `:: search-shard 1` section contains 1..128 childless `:: data HEX`
sections. HEX is lowercase, even-length hexadecimal. Each leaf encodes 768
bytes, except the final leaf which may be shorter. A shard decodes to 98,304
bytes except the final shard, which completes `bytes`. Raw UTF-8 may cross shard
boundaries: concatenate bytes before decoding. Hex encoding keeps data inert,
avoids YLang inline escaping ambiguity and stays below its 4096-byte line limit.
The production YLang parser enforces 256 KiB/document, 4096 nodes and depth 32;
the search validator accepts only this much smaller exact AST shape.

## Payload

The concatenated data is JSON with only these fields:

```json
{"version":1,"records":[{"page":{"key":"home","title":"Example","headings":"Heading","text":"Original visible text","address":"yami://FULL-NODE-ID/home"},"terms":["example","heading","original","text","visible"]}]}
```

Every address must be canonical, contain a complete checksummed identity and
have exactly the record's safe document key. Records must be strictly ordered
by canonical address. Duplicate addresses, even identical records, are invalid
within a publication. Original title/headings/text are retained for weighted
search and matching snippets. No separately supplied snippet can override them.
Controls other than newline are invalid. Titles are nonempty and <=4096 UTF-8
bytes. All three fields together are <=65,536 bytes/page, <=4,194,304 total.
Unsafe platform device keys and key/path traversal are invalid.

`terms` is the sorted, unique token set derived from all three original fields:
Unicode NFKD (unicode-normalization 0.1.26), then the shared Discover fold,
split into alphanumeric runs. Latin accent/case folding follows Discover;
scripts without spaces retain substring matching. Receivers MUST recompute
and compare terms and rebuild postings. Tokens <=256 bytes, vocabulary <=65,536
and postings <=262,144. Reject unknown/duplicate JSON fields, invalid UTF-8,
invalid records or unsupported versions; do not silently salvage a subset.
Consumers must bound the input before parsing. Future incompatible folding,
record or envelope changes require a new format version.

## Publication and trust

Builds are deterministic. Shard hashes prevent mixing generations and detect
incomplete transfers; they do not prove a provider's statements about a site.
TLS authenticates the exact provider for every manifest/shard request. Normal
navigation separately authenticates the destination website. Queries execute
locally and are never included in publication requests.

Provider import is explicit and requires operator authorization for the supplied
content. Identical addresses across explicit imports deduplicate only if all
page fields agree; conflicting duplicates fail without modifying provider state.
Export requires a new directory, never overwrites existing site documents, and
writes the manifest last. Stop serving before switching publication directories.
No automatic crawling, background fetching or public exposure is performed.
