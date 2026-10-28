# YamiNet

An experimental, privacy-first alternative web developed in Rust.

## Planned components

- **YLang (`.yl`)** — original text-document language and parser.
- **YSL (`.ysl`)** — native styling and layout definitions.
- **YNP** — custom application protocol, independent of HTTP and HTTPS.
- **Yami Browser** — native browser and renderer without HTML or web engines.
- **Yami Node** — peer-hosted content and cryptographic identities.

## Development roadmap

1. Create the Rust workspace and implement YLang parsing and tests.
2. Add YSL and a native document renderer.
3. Implement YNP framing and a two-node localhost demonstration.
4. Add authenticated encryption and peer identity verification before network exposure.
5. Later, investigate metadata-resistant, pluggable routing after the foundation works.

> **Security status:** Early prototypes are experimental and must not be used for sensitive or anonymous communications. Direct connections expose network metadata. Do not expose unencrypted localhost prototypes to the internet.

## Principles

No HTML, CSS, JavaScript, DNS or HTTP in the YamiNet application architecture. Reuse established, reviewed cryptographic primitives rather than inventing encryption algorithms. Keep network transport replaceable to support stronger anonymity systems in the future.
