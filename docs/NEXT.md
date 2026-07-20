# emend-lt — next steps (session handoff)

Standalone Rust reimplementation of LanguageTool's engine, reading LT's own data files and
serving the same `/v2/check` HTTP contract. **English-first.** Oracle for every step is bundled
**Java LanguageTool 6.6** in `~/repositories/scriptorium/packages/desktop/resources/languagetool`.

## Done & verified byte-exact vs Java LT 6.6

- **Phase 0** — target rule scope enumerated (`docs/phase0-scope.md`, `tools/phase0/`).
- **1.1 Morfologik CFSA2 `.dict` reader** (`crates/morfologik`) — POS 259,353 + spelling 206,845 seqs, byte-exact vs `fsa_decompile`.
- **1.2 SRX sentence splitter** (`crates/segmenter`) — 40 probes identical to `SRXSentenceTokenizer`.
- **1.2 word tokenizer** (`crates/tokenizer`) — 58 sentences identical (needs the tagger).
- **1.3 POS tagger** (`crates/tagger`) — 3,025 words identical to `EnglishTagger`.
- **Shared token-pattern matcher** (`crates/matcher`) — token+sequence match, XML + `<!ENTITY>`
  expansion; **80.1% grammar / 80.4% disambiguation** patterns fully supported (down from a
  previously-reported 84.7/82.2% because `exception scope=` and uncompilable regexes are now
  *flagged* instead of silently mis-run). 16 tests pass.
- **1.3b Disambiguator action layer** (`crates/analysis`) — parses `en/disambiguation.xml`
  (rule/rulegroup + `<disambig>` + `<wd>` + `<antipattern>`), applies the actions in document
  order over the analyzed sentence. **Raw analysis (`getRawAnalyzedSentence`) is byte-identical**
  to Java LT; post-disambiguation **token-reading parity ≈ 85.9%** over a 1,500-sentence corpus,
  the gap being the same deferred matcher features (chunk / and-or / scope-exceptions), with
  ~zero genuine false positives. 9 tests pass.

Details per phase: `docs/phase1-english.md`. Nothing is committed yet (still on `main`).

## Do next, in order

- [x] **1. Disambiguator action layer** on top of `crates/matcher` (completes 1.3). **DONE.**
  Lives in `crates/analysis` (`Analyzer::raw` + `disambig::{parse_disambig_rules, apply_all}`).
  Actions implemented: `replace`, `add`, `remove`, `filter`, `filterall` (+ `unify`/
  `ignore_spelling`/`immunize` as reading-no-ops, which is what LT does to lemma/POS). Oracle:
  `oracle/AnalyzedOracle.java` (mode `raw`/`dis`, isolates `XmlRuleDisambiguator`), diffed by
  `crates/analysis/src/bin/analyze.rs`. See phase1-english.md §1.3b for the exact action semantics
  and gotchas (they bit us repeatedly).
- [ ] **2. `<and>`/`<or>` token groups** in the matcher (49 grammar / 111 disambig patterns). Cheap;
  lifts coverage. **Also implement `<exception scope="previous|next">`** (313 grammar / 33 disambig
  patterns now flagged unsupported) and the OpenNLP **chunker** — those three are the whole
  disambiguation parity gap.
- [ ] **3. Phase 1.4 grammar engine** on the same matcher: `<rule>` + `<message>` + `<suggestion>` + `<match>` + antipatterns; emit `matches[]`. Start with the Phase 0 top English rules (`MODAL_OF`, `THERE_THEIR`, `BASE_FORM`, `EN_A_VS_AN`, …). Oracle = `POST /v2/check`.
- [ ] **4. Hand-coded English rules** (1.5): `EN_CONTRACTION_SPELLING`, `EN_A_VS_AN`, `SPURIOUS_APOSTROPHE`, etc. (see `docs/phase0-scope.md`).
- [ ] **5. Chunker** (`chunk`/`chunk_re`, 818 grammar patterns) — OpenNLP phrase chunker, a separate subsystem. Deferrable: Phase 0 fired rules skew away from chunk-heavy style rules.
- [ ] **6. Phase 2** — wrap in an `axum`/`hyper` server exposing `/v2/check` + `/v2/languages`, honoring `disabledRules`/`disabledCategories` and echoing `--allow-origin`.
- [ ] **7. Phase 3** — differential harness driving `/v2/check` parity % as the headline metric.

## Key findings to remember (things that bit us / corrections to the roadmap)

1. `.dict` files are Morfologik **CFSA2** (magic `\fsa`, ver `0xC6`) — **not** an `fst`-crate format. Ported from `morfologik.fsa.CFSA2`.
2. English **word tokenizer calls the POS tagger** (`wordsToAdd` → `isTagged`), so 1.2⇄1.3 interlock; tagger injected as `tokenizer::WordTagger`.
3. Tagger = **(dict ∪ `added.txt`) − `removed.txt`**, no dedup (e.g. `climbdowns` doubles).
4. `URL_CHARS` in LT is a **range `$`–`_`** (covers `:` `=` `?` …) — do not escape the hyphen. (Bug the oracle caught.)
5. Rule files use **`<!ENTITY>` regex-macros** (`&uncommon_verbs;` ×169) that quick-xml won't expand — must pre-expand the whole file (`crates/matcher/entities.rs`).
6. Disambiguation + grammar share the **same token-pattern matcher** — build once (done), drive both.

## Environment / reproduction

- **Rust**: `cargo build --release`, `cargo test` (workspace).
- **Java LT 6.6 oracle** (loose classes under `lt/`, dicts inside `libs/english-pos-dict.jar`):
  ```sh
  LT=~/repositories/scriptorium/packages/desktop/resources/languagetool/lt
  JRE=~/repositories/scriptorium/packages/desktop/resources/languagetool/jre-darwin-arm64/bin
  CP="$LT:$LT/languagetool-server.jar:$LT/libs/*"
  # dicts must be extracted first: unzip english-pos-dict.jar; copy added.txt/removed.txt next to english.dict
  ```
- **Running LT HTTP server** (for `/v2/check` oracle):
  `"$JRE/java" -cp "$CP" org.languagetool.server.HTTPServer --port 8099 --allow-origin "*"`.
- Java oracle sources + extracted dicts currently live in the session scratchpad; **regenerate
  them** in the new session (extract `english.dict`/`.info` from `libs/english-pos-dict.jar`,
  copy `en/added.txt` + `en/removed.txt` beside it). Existing oracle Java files to recreate:
  `TagOracle.java`, `SentOracle.java` (patterns shown in `docs/phase1-english.md`).

## App integration contract (Phase 4 target, unchanged)

Renderer hits `localhost:8010`: `GET /v2/languages` (checks `response.ok` + echoed
`access-control-allow-origin`) and `POST /v2/check` (form: `text`, `language`, `level=default`,
`disabledRules`, `disabledCategories`; reads only `matches[].{offset,length,message,
replacements[].value,rule.id,rule.category.id}`). Config lives in Scriptorium
`languagetool-linter.ts` (disabled sets) and `languagetool.ts` (spawn).
