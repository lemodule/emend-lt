# Phase 1.1 — Morfologik FSA reader (English)

Status: **done and verified.** The first and most load-bearing engine primitive —
reading LanguageTool's `.dict` files — works for English, backing both spelling
(`MORFOLOGIK_RULE_EN_US`) and POS tagging.

## What was built

`crates/morfologik` — a from-scratch Rust reader for the Morfologik **CFSA2** FSA format
(`crates/morfologik/src/fsa.rs`) plus a dictionary layer (`src/dict.rs`) that parses the
sibling `.info` metadata and does:

- **spelling membership** — `Dictionary::contains(word)`
- **POS lookup** — `Dictionary::lookup(word) -> [(stem, tag)]`, decoding the SUFFIX-encoded
  base forms.

Two CLIs: `fsa-dump` (enumerate every accepted sequence) and `dict-lookup`.

## Key finding that corrects the roadmap

The roadmap assumed the `fst` crate would cover the FSA reader. It does **not**: LT's
`.dict` files are Morfologik **CFSA2** (magic `\fsa`, version byte `0xC6`), which is a
different on-disk format from what the `fst` crate implements. The reader was ported
directly from the reference `morfologik.fsa.CFSA2` Java class instead. This is a one-time
cost already paid; the same reader serves every language's dictionaries.

## Verification (reproducible)

The English dicts live **inside** `libs/english-pos-dict.jar` in the Scriptorium bundle
(not extracted to disk — a note for Phase 4 packaging). Extract them, then:

1. **FSA reader vs. Morfologik's own Java decompiler** — the strongest possible oracle.
   Dump every accepted byte sequence with both and compare as a multiset:

   | Dict | Sequences | Rust == Java |
   |---|---|---|
   | `english.dict` (POS) | 259,353 | ✅ byte-exact |
   | `en_US.dict` (spelling) | 206,845 | ✅ byte-exact |

   ```sh
   JAVA -cp "libs/morfologik-tools.jar:libs/*" morfologik.tools.Launcher \
       fsa_decompile -i english.dict -o java.txt
   cargo run --release -p morfologik --bin fsa-dump -- english.dict > rust.hex
   # hex-encode java.txt lines, sort both, diff  (see tools/phase0 pattern)
   ```

2. **Spelling membership** — `dict-lookup en_US.dict contains …`: real words `true`; the
   actual Phase 0 corpus misspellings `becuase`/`occured` `false`.
3. **POS SUFFIX decode** — `dict-lookup english.dict lookup …`: `walked→walk` (VBD/VBN,
   raw entry `walked+C+VBD`), irregulars `children→child`, `mice→mouse`, `better→good/well`.
   Locked in `cargo test -p morfologik` (SUFFIX decoder unit tests).

## Next (English, roadmap order)

- 1.2 Tokenizer + SRX sentence splitter
- 1.3 POS tagger + disambiguator (uses this reader + `disambiguation.xml`)
- 1.4 XML pattern-rule engine (`en/grammar.xml`) — the long pole
- 1.5 The handful of hand-coded English rules (`EN_CONTRACTION_SPELLING`, `EN_A_VS_AN`, …)
