# emend-lt

A from-scratch **Rust** grammar/spell-checking engine that reads LanguageTool's own rule
and dictionary files unchanged and serves the same local HTTP API (`/v2/check`,
`/v2/languages`), so an app embedding LanguageTool's Java server can swap it in with no
client-side changes and no JVM. Not affiliated with, endorsed by, or a fork of the
LanguageTool project. LGPL-2.1.

See the roadmap in [`languagetool-rust.md`](languagetool-rust.md).

## Status

- **Phase 0 — scope**: done. See [`docs/phase0-scope.md`](docs/phase0-scope.md).
- **Phase 1.1 — Morfologik FSA reader (English)**: done, verified byte-exact vs Morfologik's
  own Java decompiler.
- **Phase 1.2 — SRX sentence splitter + word tokenizer (English)**: done, both verified
  byte-identical to LT (sentence splits over 40 probes; word tokenization over 58 sentences).
- **Phase 1.3 — POS tagger (English)**: done, verified byte-identical to LT's `EnglishTagger`
  over 3,025 words.
- **Shared token-pattern matcher**: the `<pattern>`/`<token>` core that both
  `disambiguation.xml` (1.3b) and `grammar.xml` (1.4) compile to — token + sequence matching,
  XML + `<!ENTITY>` expansion. Validated against real English files: ~84% of patterns fully
  supported today; the rest use `chunk` (needs the OpenNLP chunker) or `<and>`/`<or>` groups.

See [`docs/phase1-english.md`](docs/phase1-english.md). Focus is **English-first** to reach
end-to-end parity as a proof point before other languages.

## Layout

- `crates/morfologik` — CFSA2 `.dict` reader + dictionary lookup (spelling + POS).
- `crates/segmenter` — SRX sentence segmentation (`segment.srx`).
- `crates/tokenizer` — English word tokenizer (tagger injected as a trait).
- `crates/tagger` — English POS tagger (Morfologik + `added.txt`/`removed.txt`).
- `crates/matcher` — shared token-pattern matcher for disambiguation + grammar rules.
- `tools/phase0` — rule-scope enumeration harness.
- `docs/` — per-phase results.

## Build & test

```sh
cargo build --release
cargo test -p morfologik
```
