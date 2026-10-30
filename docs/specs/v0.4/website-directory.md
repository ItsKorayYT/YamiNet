# Website directory format 1 (YamiNet 0.4 draft)

Status: **draft, local-only**. This document specifies the version-1 directory
document read by `yami-discover`. It adds data, not networking: no server,
crawler, account, telemetry or new wire protocol is defined here.

## 1. Purpose

A directory is an independent, operator-maintained list of YamiNet websites.
It exists so a user can find a site without already knowing its full `yami://`
address. Directories are ordinary documents that can be copied, reviewed,
diffed, mirrored and imported offline.

A directory entry is an **untrusted hint**. It does not sign, certify, rank for
security, or authenticate the listed destination. Opening the destination still
requires the normal YamiNet identity authentication over TLS 1.3. A directory
operator can list any identity, describe it inaccurately, omit sites or
disagree with another operator.

## 2. Container format

A version-1 directory is a valid **YLang 0.2** document. It reuses the existing
document parser, metadata, spans, limits, inline text and section/container
nodes. It has this shape:

```ylang
@page 0.2
  title YamiNet Community Directory
  language en

:: directory 1
  :: card Alice's Notebook
    > yami://yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re/home | Open Alice's Notebook
    Field notes on local-first software, identity and small YamiNet networks.
    :: category Personal
```

Rules:

1. The first line must be `@page 0.2`. YLang 0.1 documents are not directory
   documents.
2. The `title` metadata is the directory display name. `language` is optional
   and currently informational.
3. Exactly one top-level block is allowed: the `:: directory 1` section. Other
   top-level headings, paragraphs, links or sections are rejected.
4. The section name must be exactly `directory` followed by the canonical
   decimal format version. Version `1` is the only version this build reads.
   A future version will use `:: directory 2` and must be rejected rather than
   guessed at by this build.
5. Children of the wrapper are entries, in document order.

Because the document is YLang, the existing 256 KiB source limit, 4096-node
limit, 32-level nesting limit, control-character rules and line limits apply
before the directory rules below.

## 3. Entry format

Each entry is a YLang `:: card <title>` container:

```ylang
  :: card <title>
    > yami://<node-id>/<key> | <link label>
    <short description>
    :: category <category>
```

Rules:

- `<title>` is required, trimmed and limited to 120 Unicode scalar values.
- Exactly one YLang link is required. Its target must be a canonical
  `yami://` address. The link label is required by YLang but is not part of the
  directory entry.
- Exactly one description paragraph is required. Adjacent YLang source lines
  form one paragraph; a blank line starts a second paragraph, which is
  rejected. Inline YLang markup (`[strong: ...]`, `[em: ...]`, `[code: ...]`)
  is accepted and reduced to plain text for the description. The description
  is trimmed and limited to 320 Unicode scalar values.
- Zero or one `:: category <category>` section is allowed. It must have no
  child blocks. The category is trimmed and limited to 48 Unicode scalar
  values.
- Headings, images, lists, separators, nested cards/rows/groups and unknown
  sections inside an entry are rejected.
- The order of the link, description and category inside the card is free.

The title, description and category are operator-supplied display text. They
are not sanitized HTML and are never executed; there is no HTML, CSS,
JavaScript or external web engine in this format.

## 4. Address canonicalization

An address has the form:

```text
yami://<node-id>[/<key>]
```

- `<node-id>` must be the complete lowercase `yn1-` identity: 58 lowercase
  RFC 4648 base32 characters (a-z, 2-7) plus the 4-byte checksum. Parsing uses
  the existing `yami-identity` node ID parser, including Ed25519 point checks.
- `<key>` follows the existing YNP document-key rules: 1 to 128 ASCII letters,
  digits, `_` or `-`.
- An omitted key, an empty key and a trailing slash normalize to `home`.
- The canonical form always includes the key: `yami://<node-id>/<key>`.
- Schemes other than `yami://`, uppercase schemes, uppercase node IDs,
  whitespace, query strings, fragments and extra path segments are rejected.

The canonical address is also the deduplication key. `yami://<id>` and
`yami://<id>/home` are the same entry. Two different keys on the same node are
different entries.

Address validation proves only that the text names a syntactically valid node
identity and document key. It proves nothing about who controls that identity,
whether the document exists, or whether the page is safe.

## 5. Validation limits

| Resource | Limit |
| --- | --- |
| Directory source size | 256 KiB (YLang `MAX_SOURCE`) |
| Entries per directory | 512 |
| Unique entries per merged catalog | 2048 |
| Independent sources per catalog | 32 |
| Directory display name | 80 characters |
| Entry title | 120 characters |
| Entry description | 320 characters |
| Entry category | 48 characters |
| Source label | 80 characters |
| Search query | 128 characters |
| Search terms per query | 8 |
| Search results returned | 100 |

Limits are enforced while parsing or importing so malformed or resource-abusing
documents fail with an error instead of allocating without bound. A failed
import does not partially modify the catalog.

## 6. Merging and duplicates

A catalog merges sources in import order. Source labels are chosen by the
importer and must be unique and non-empty. The first entry for a canonical
address wins; later entries with the same canonical address are counted as
duplicates and do not replace the first entry's title, description, category or
source. Duplicates inside one document are handled the same way.

Import reports expose the source display name, version, number of listings
read, number added and number of duplicates skipped. Deduplication is
deterministic and local. It is an editorial convenience, not a trust or
freshness decision.

## 7. Local search

Search runs entirely in the importing process over the merged entries. The
query, the results and the fact that a search happened are never sent to a
directory operator or any other peer.

- Matching is substring-based, so partial words match.
- Matching is case-insensitive and folds common Latin diacritics, so `CAFÉ`,
  `café` and `cafe` match the same title. `Straße` and `strasse` match, and
  `İstanbul` folds to `istanbul`.
- Non-Latin scripts are matched exactly; for example, `日本` matches a Japanese
  title or description.
- Multiple whitespace-separated terms are combined with AND: every term must
  match somewhere in the entry (title, description, category or address
  without the `yami://` scheme).
- Ranking favors title matches, then category, description and address
  matches. Exact and prefix title matches receive extra weight. Ties are
  ordered by folded title, canonical address and source label.
- An empty query lists entries in deterministic order. Results are capped at
  100. An explicit limit is clamped to that cap.

The search index is built at import time and bounded by the entry and field
limits above.

## 8. Security considerations

Directories are a discovery aid, not a public-key infrastructure.

- **No trust transfer.** Listing an entry does not mark the destination as
  verified, trusted, safe or owned by anyone. Browser integration must label
  results as untrusted until the existing TLS 1.3 identity check succeeds.
- **No signature requirement.** Anyone can publish a directory. A directory
  can be malicious, stale, incomplete or Sybil-stuffed. Version 1 does not
  define directory signatures or reputation.
- **Identity pinning still happens at open time.** The existing networking
  stack must connect only to the exact node ID in the canonical address and
  authenticate it over TLS 1.3, directly or through the existing relay path.
- **No automatic crawling or fetching.** Importing a directory reads one local
  file. Opening an entry is an explicit user action in a future browser
  milestone.
- **No query leakage.** Local search never contacts a directory operator.
- **No new protocol.** This format is inert content. It does not change TLS,
  DHT, relays, discovery multicast or browser security.
- **Conflicting listings are possible.** The first imported source wins for
  metadata. That is a deterministic merge rule, not a claim that the first
  listing is more accurate.

## 9. Compatibility and versioning

- Directory version 1 requires YLang 0.2. The existing YLang/YSL 0.1 and 0.2
  document formats are unchanged.
- YSL is not needed. A directory can be displayed with the normal YLang reader,
  but this milestone does not add a browser UI.
- A future directory version must use a new wrapper name (`:: directory N`) and
  a new specification. This build returns an explicit unsupported-version error
  rather than silently accepting unknown fields.
- Unknown YLang directives are already rejected by the YLang parser. Unknown
  content inside a version-1 entry is rejected by the directory validator.
