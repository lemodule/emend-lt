# emend-lt

A from-scratch **Rust** grammar/spell-checking engine that reads LanguageTool's own rule
and dictionary files unchanged and serves the same local HTTP API (`/v2/check`,
`/v2/languages`), so an app embedding LanguageTool's Java server can swap it in with no
client-side changes and no JVM. Not affiliated with, endorsed by, or a fork of the
LanguageTool project. LGPL-2.1.

See the roadmap in [`languagetool-rust.md`](languagetool-rust.md).

## Status

- **Phase 0 — scope**: done. See [`docs/phase0-scope.md`](docs/phase0-scope.md).
- **Phase 1.1 — Morfologik FSA reader (English)**: done and verified byte-exact against
  Morfologik's own Java decompiler. See [`docs/phase1-english.md`](docs/phase1-english.md).

Focus is **English-first** to reach end-to-end parity as a proof point before other
languages.

## Layout

- `crates/morfologik` — CFSA2 `.dict` reader + dictionary lookup (spelling + POS).
- `tools/phase0` — rule-scope enumeration harness.
- `docs/` — per-phase results.

## Build & test

```sh
cargo build --release
cargo test -p morfologik
```
