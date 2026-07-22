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
- **Shared token-pattern matcher** (`crates/matcher`) — token+sequence match, `<and>`/`<or>`
  groups, `<exception scope="previous|next">`, XML + `<!ENTITY>` expansion; **85.2% grammar /
  91.7% disambiguation** patterns fully supported. The **only** remaining unsupported feature is
  the OpenNLP `chunk`/`chunk_re` (818 grammar / 86 disambig). 18 tests pass.
- **1.3b Disambiguator action layer** (`crates/analysis`) — parses `en/disambiguation.xml`
  (rule/rulegroup + `<disambig>` + `<wd>` + `<antipattern>`), applies the actions in document
  order over the analyzed sentence. **Raw analysis (`getRawAnalyzedSentence`) is byte-identical**
  to Java LT; post-disambiguation **token-reading parity ≈ 89.6% / sentence parity 32%** over a
  1,500-sentence corpus. The whole remaining gap is the **chunker**: either a rule that reads a
  `chunk` (skipped), or a *cascade* — a supported rule mis-firing because a chunk-based rule
  upstream didn't run to narrow the readings it sees. 11 tests pass.
- **1.4 Grammar engine** (`crates/analysis` `grammar.rs`) — `grammar.xml` `<rule>`s on the same
  matcher, emitting `/v2/check` `matches[]`. **3,916 of 5,529 rules supported** (rest need Java
  `<filter>` / chunker). **96.1%** of supported rules pass their own `<example>` cases (99.0% of
  negative examples). 18 tests pass.
- **1.4b `<match>` in `<suggestion>`/`<message>`** (`grammar.rs`) — non-synthesizer variants:
  plain `<match no=N>` (== `\N`), `case_conversion` (all/start · upper/lower · preserve), and
  `regexp_match`→`regexp_replace`. This is what lifted supported rules 3,159 → 3,789. Semantics
  pinned against LT bytecode (`javap -c` on `MatchState`/`PatternRuleMatcher`): out-of-range `no`
  **clamps to the last matched token**; empty `min="0"` backrefs render empty with double-space
  collapse; Java `$1`-adjacency (`$1re`) translated to fancy-regex `${1}re`; outer
  capitalize-on-uppercase-error is suppressed iff the suggestion opens with `\N` **and** a match
  converts case (`matchPreservesCase`).
- **1.4c Morfologik synthesizer** (`crates/morfologik` `synth.rs` + `grammar.rs`) — the inverse of
  the tagger: `english_synth.dict` (+ `english_tags.txt`) maps `lemma|postag` → surface form (the
  same CFSA2 SUFFIX decode, run on the `lemma|postag` key). Wired into `<match postag=…>` rendering
  (`apply_match_spec`/`synthesize_forms`), reproducing LT `MatchState.toFinalString` +
  `BaseSynthesizer`: a **concrete** tag (`postag="VB"`) synthesizes each reading's lemma; a **regexp**
  tag (`postag_regexp="yes"` [+ `postag_replace`]) rewrites the token's own POS tag then iterates the
  tag universe; a **static lemma** (`<match …>toe</match>`) inflects the given lemma as the token is.
  Multi-form results expand a `<suggestion>` into the **cross-product** of alternatives; an empty
  result falls back to the token surface (LT). This lifted supported rules **3,789 → 3,916**. Tag
  filtering is a **full-string** match (LT `Matcher.matches()` is anchored — a partial search
  mis-synthesized `NNS` back from `NN(:UN?)?`). Still flagged `match-synth`: the `+DT`/`+INDT`
  "insert an article" and `_spell_number_` modes.
- **1.5 Spelling — `MORFOLOGIK_RULE_EN_US` detection** (`crates/morfologik` `speller.rs` +
  `crates/analysis` `spelling.rs`). `Speller::is_misspelled` is the **byte-exact** port of
  `morfologik.speller.Speller.isMisspelled` for `en_US` (frequency-included, case-converting): not in
  `en_US.dict`, and — unless mixed-case — its lowercase is unknown too; digit-bearing words ignored.
  Validated **0 diffs / 420 words** vs the live server. The rule layer (LT `SpellingCheckRule`) adds
  token acceptance: skip punctuation/number/URL/e-mail/`@`·`#`·`$` tokens; the `ignore.txt` /
  `spelling*.txt` accept-lists; `prohibit.txt` always-flag; **hyphen-compound** acceptance (every
  `-`-part a known word); and `multiwords.txt` (each capitalized term-word spelling-ignored — this is
  where proper nouns like `Django` come from). Detection parity vs the live `/v2/check`: **46/47
  exact spans (97.9%)** over a mixed corpus, **zero** false positives on clean prose, the one miss
  being LT's run-on multi-word *suggestion* span (`novell about`). Suggestions are a best-effort
  edit-distance-1→2 dictionary search ranked by stored frequency — **not** the full Morfologik
  speller (bounded FSA-Levenshtein + replacement pairs), so their ordering/coverage is approximate.

- **Phase 2 — HTTP server** (`crates/server`, `bin/emend-server`) — wraps the whole pipeline behind
  LanguageTool's `/v2` contract with `axum`. An `Engine` loads every data file **once**
  (tagger, disambiguation rules, grammar rules, segmenter, optional synthesizer, optional spelling
  rule) and `Engine::check` runs the exact per-sentence pipeline the `check-text`/`spell-text` bins
  do (segment → raw analyze → disambiguate → grammar (+synth) → spelling), **merging** grammar and
  spelling matches, sorting document-order. `POST /v2/check` (form: `text`, `language`, `level`,
  `disabledRules`, `disabledCategories`) emits the LT JSON schema — the app's load-bearing
  `matches[].{offset,length,message,replacements[].value,rule.id,rule.category.id}` plus a computed
  `context`; `disabledRules`/`disabledCategories` filter post-hoc. `GET /v2/languages` returns the
  English set. `--allow-origin` echoes `Access-Control-Allow-Origin` (+ preflight). Validated against
  the **live Java LT 6.6 server** (running as oracle on `:8010`): over a mixed corpus **100%
  precision (0 false positives) / ~71% recall** — every span emitted is one Java also flags; the
  misses are the not-yet-ported hand-coded rules (`EN_A_VS_AN`, `EN_CONTRACTION_SPELLING`, …, #4) and
  chunker-dependent rules (#3), i.e. false *negatives* by design. Server output for the grammar layer
  is identical to `check-text`. 4 unit tests. `--data-dir <DIR>` fills every path from the
  conventional filenames, so a fully-extracted dir needs only `--data-dir`.

Details per phase: `docs/phase1-english.md`. Nothing is committed yet (still on `main`).

## Done (this phase)

- [x] **1.3b Disambiguator action layer** — see summary above / phase1-english.md §1.3b.
- [x] **`<and>`/`<or>` groups + `<exception scope="previous|next">`** in the matcher.
- [x] **1.4 grammar engine MVP** — `\N` backrefs, antipatterns, `default=off`/`picky`, overlap
  filtering, case-preservation; 95.9% precision vs `/v2/check`.
- [x] **1.4b `<match>` in suggestions/messages** (non-synthesizer variants) — see summary above.
  Also fixed the oracle to skip `type="triggers_error"` examples (LT documents these as
  expected-to-fire, not negatives; 203 of them).
- [x] **1.4c Morfologik synthesizer** — `<match postag=…>` (concrete / regexp / `postag_replace` /
  static-lemma), cross-product suggestion expansion. +127 rules (3,789 → 3,916). See summary above.
- [x] **1.5 Spelling detection** (`MORFOLOGIK_RULE_EN_US`) — byte-exact `is_misspelled` (0/420 vs
  server) + rule layer; 46/47 exact spans, 0 FP. Suggestions best-effort ED1→2. See summary above.
- [x] **Phase 2 HTTP server** (`crates/server`, `bin/emend-server`) — `axum` `/v2/check` +
  `/v2/languages`, load-once `Engine`, grammar+spelling merged, `disabledRules`/`disabledCategories`,
  `--allow-origin`. 100% precision / ~71% recall vs live Java LT. See summary above.

## What's left, in priority order

- [ ] **1. `<match>` element** — sub-items (a) suggestions/messages (1.4b) and (b) the **synthesizer**
  (1.4c) are **done**; what remains:
  - [x] **in `<suggestion>`/`<message>`, non-synthesizer** — plain / `case_conversion` /
    `regexp_match`+`regexp_replace`. Landed (1.4b).
  - [x] **the synthesizer** — `<match postag=… [postag_replace=…]>` + static-lemma, over
    `english_synth.dict`. Landed (1.4c, +127 rules). Remaining synth gap: the `+DT`/`+INDT`
    "insert an article" mode (33 uses) and `_spell_number_` — both still flagged `match-synth`.
  - [ ] **in `<token>`** — token-level backreference (`<token><match no='0'/></token>`): the
    position must equal an earlier matched token. Still flagged `token-match`, skipped
    (`crates/matcher/parse.rs`).
  - [ ] **in `disambiguation.xml`** — the same `<match>` (2 disambig rules deferred).
  - Oracle: `bin/grammar-examples` (self `<example>` corpus, now takes optional
    `<english_synth.dict> <english_tags.txt>` args) + `bin/check-text` vs the live server.
- [ ] **2. Java `<filter>` classes** (71 grammar rules) — post-match filters currently flagged and
  skipped. Port the high-value ones (`MultitokenSpellerFilter`, then the handful of others). Needs
  the spelling dict (`en_US.dict`, already read by `crates/morfologik`).
- [ ] **3. OpenNLP phrase chunker** (`chunk`/`chunk_re`, 818 grammar / 86 disambig) — the **last
  matcher feature** and the entire remaining *disambiguation* parity gap (89.6% → higher; also
  removes the cascade false-positives). Separate ML subsystem: OpenNLP maxent models in
  `libs/opennlp-chunk-models.jar` (GIS/maxent format + feature extraction). Biggest single effort;
  deferrable per Phase 0.
- [ ] **4. Hand-coded English rules** (1.5): the Java rules not expressible in XML —
  `EN_CONTRACTION_SPELLING`, `EN_A_VS_AN`, `SPURIOUS_APOSTROPHE`, etc. (`docs/phase0-scope.md`).
- [x] **5. Spelling detection** (`MORFOLOGIK_RULE_EN_US`) — landed (1.5): byte-exact `is_misspelled`
  + rule layer, 46/47 exact spans / 0 FP vs live. **Remaining spelling work** (optional polish):
  - [ ] **Morfologik speller suggestions** — port the real `morfologik.speller.Speller` (bounded
    FSA-Levenshtein ED2 + `replacement-pairs` + frequency weighting; 22 KB of bytecode) to make
    `replacements[]` byte-exact. Current suggestions are approximate ED1→2. Also gives LT's
    **run-on multi-word** suggestion span (`novell about`), the one detection-span diff.
  - [ ] **`MultitokenSpellerFilter`** (#2) now unblocked — it reuses this speller.
- [x] **6. Phase 2 — HTTP server** (`crates/server`, `bin/emend-server`) — **landed.** `axum`
  `POST /v2/check` + `GET /v2/languages`, `disabledRules`/`disabledCategories`, `--allow-origin`.
  See summary above. `level` is read but a no-op today: `level=picky` would enable the
  `default="off"`/`tags="picky"` rules, which the engine currently **compiles out as unsupported**
  (`grammar.rs` flags them `disabled`) — to serve picky, split that flag from `unsupported` and gate
  it on the request level. The app only sends `level=default`. Also **not yet done**: cross-subsystem
  overlap filtering (grammar's `filter_overlaps` runs within grammar; spelling matches are merged
  but not de-overlapped against grammar — LT does this globally), and UTF-16 offsets (matches use
  Unicode-scalar char offsets, identical to LT for BMP/English; non-BMP like emoji would drift).
- [ ] **7. Phase 3 — differential harness** — drive `/v2/check` parity % (precision/recall over a
  corpus) as the headline metric. Prototyped: the parity script diffs `emend-server` vs the live
  Java LT server span-for-span (100% precision / ~71% recall on the smoke corpus). **Next:** promote
  it to a checked-in bin/test over a real corpus, tracking precision/recall/rule-coverage per run.

**Recommended next:** the **differential harness** (#7) — the HTTP server (#6) just landed and is
already diffed ad-hoc against the live Java LT server (100% precision / ~71% recall on a smoke
corpus). Promote that diff to a checked-in bin/test over a **real corpus**, reporting
precision/recall/rule-coverage each run, so every subsequent change (porting a `<filter>`, a
hand-coded rule, the chunker) is measured as a parity delta rather than a per-rule example rate.
With the harness in place, the highest-recall wins are the **hand-coded English rules** (#4 —
`EN_A_VS_AN`, `EN_CONTRACTION_SPELLING`, … are the bulk of the current recall gap) and the **Java
`<filter>` classes** (#2). Byte-exact **spelling suggestions** (the real Morfologik speller) and the
small `<match>` tails (`+DT`, `<token>`-level) remain lower-leverage polish.

## Key findings to remember (things that bit us / corrections to the roadmap)

1. `.dict` files are Morfologik **CFSA2** (magic `\fsa`, ver `0xC6`) — **not** an `fst`-crate format. Ported from `morfologik.fsa.CFSA2`.
2. English **word tokenizer calls the POS tagger** (`wordsToAdd` → `isTagged`), so 1.2⇄1.3 interlock; tagger injected as `tokenizer::WordTagger`.
3. Tagger = **(dict ∪ `added.txt`) − `removed.txt`**, no dedup (e.g. `climbdowns` doubles).
4. `URL_CHARS` in LT is a **range `$`–`_`** (covers `:` `=` `?` …) — do not escape the hyphen. (Bug the oracle caught.)
5. Rule files use **`<!ENTITY>` regex-macros** (`&uncommon_verbs;` ×169) that quick-xml won't expand — must pre-expand the whole file (`crates/matcher/entities.rs`).
6. Disambiguation + grammar share the **same token-pattern matcher** — build once (done), drive both.
7. quick-xml emits `&quot;`/`&#39;` as separate **`GeneralRef` events**; if unhandled, a
   `<token>&quot;</token>` silently becomes a match-anything token (corrupted whole sentences /
   fired a rule 360× on random text). Same trap for **`<token><match/></token>`** and **failed
   regex compiles** — all three now flagged unsupported instead of matching everything.
8. Grammar `matches[]` gotchas (all reproduced): a bare `<disambig postag>` touches only the first
   marker token; a `<suggestion>` can be a **direct child of `<rule>`** (not only inside
   `<message>`); LT **capitalizes suggestions** when the match is uppercase-initial; LT **drops a
   match contained in another** match's span; `default="off"` / `tags="picky"` rules are off at
   `level=default`.
9. **Never silently mis-apply.** Every unsupported feature (chunk, `<match postag>`, `<filter>`,
   scope before it landed) is flagged per-rule and skipped, so gaps are false *negatives*, not wrong
   output. Coverage numbers went *down* when we started flagging honestly — that's correct.
10. **`<match>` semantics are only in bytecode** (LT ships no `.java`; `javap -c` on
    `MatchState`/`PatternRuleMatcher` is the authority). Four that bit us: (a) an out-of-range `no`
    **clamps to the last matched token** — `THE_DUTCH` uses `no="2"` on a 1-token pattern; (b) a
    `min="0"` element that matched nothing renders an **empty** backref, and the resulting double
    space is collapsed (not trimmed — a leading/trailing single space can be intentional, e.g.
    `LC_AFTER_PERIOD`); (c) Java replacement `$1re` = group 1 + literal `re`, but fancy-regex reads
    `1re` as a *group name* → translate numbered groups to `${1}`; (d) the outer
    capitalize-on-uppercase-error is suppressed **only** when the suggestion opens with a `\N`
    backref *and* some match `convertsCase()` (`matchPreservesCase`) — a plain `<match no=N>` still
    capitalizes (`WERE_MD`).
11. **`type="triggers_error"` `<example>`s are expected to fire**, not negatives (203 in
    `grammar.xml`); the `grammar-examples` oracle now skips them. Before, they inflated the negative
    pass rate for *skipped* rules and deflated it once those rules became supported.
12. **The synth `.dict` is an ordinary tagger dict with keys reversed**: the key is
    `lemma + "|" + postag` and the SUFFIX-decoded value is the surface form — so `Dictionary::lookup`
    already synthesizes (feed it `lemma|postag`, read `.stem`). LT `BaseSynthesizer` semantics that
    bit us: (a) POS-tag filtering is a **full-string** `Matcher.matches()`, so the tag universe must
    be matched **anchored** (`^(?:…)$`) — fancy-regex `is_match` is a partial search and re-derived
    the *plural* `NNS` from a `NN(:UN?)?` target; (b) synthesis loops over **all** readings and pools
    into a sorted set (concrete `postag="VBN"` on "bit" needs the `VBD/bite` reading, not reading 0,
    to reach "bitten"); (c) `postag_replace` defaults to the `postag` string when absent
    (`getTargetPosTag` does `replaceAll(postag)`); (d) empty synthesis **falls back to the token
    surface** (not dropped); (e) `<match …>toe</match>` is a **static lemma** — inflect *that* lemma
    using the token's own POS tag (filtered by `postag`); (f) a suggestion with a multi-form `<match>`
    expands to the **cross-product** of alternatives. The `+DT`/`+INDT` "insert an article" mode is
    *not* morphological synthesis and stays flagged.
13. **Spelling membership is a two-line rule** once you have the FSA: `en_US.dict` is
    frequency-included + case-converting, so `is_misspelled(w)` = `!contains(w) && (isMixedCase(w) ||
    !contains(w.to_lowercase()))`, guarded by "no digit". `contains` must accept the **frequency
    tail** (a separator arc after the word), which `Dictionary::contains` already does. Things the
    *rule layer* adds beyond membership (all needed for FP parity): skip any `@`-containing token
    (LT's e-mail test is loose — `cats@dogs` too) and `#`/`$` prefixes; accept a **hyphen compound**
    when every `-`-part is known; and — the non-obvious one — **`multiwords.txt` spelling-ignores each
    capitalized term-word**, which is the *only* reason proper nouns like `Django` (from "Django
    Unchained") aren't flagged. Suggestions are a *separate* 22 KB algorithm — do **not** conflate
    detection parity (byte-exact, achievable) with suggestion parity (approximate without the port).

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
  (Note: a live Java LT instance may already be bound to `:8010` — it makes a fine oracle. Run
  `emend-server` on a **different** port to diff; localhost resolves IPv6-first, so a `:8010`
  collision can silently route to the Java one.)
- **Running our server** (`crates/server`): `cargo build --release`, then
  `./target/release/emend-server --data-dir <DIR> --port 8011 --allow-origin "*"`, where `<DIR>`
  holds the extracted files under their conventional names (`english.dict` + `.info` + `added.txt` +
  `removed.txt`, `disambiguation.xml`, `grammar.xml`, `segment.srx`, and optionally
  `english_synth.dict`/`.info` + `english_tags.txt`, `en_US.dict`/`.info` + `ignore.txt` +
  `spelling.txt` + `spelling_en-US.txt` + `multiwords.txt` + `prohibit.txt`). Individual `--dict`
  `--grammar` … flags override any convention. Parity smoke check: `POST` the same text to
  `emend-server` and the Java LT server and diff `matches[].{offset,length,rule.id}`.
  Note: JDK 17 lives at `/usr/bin/javac`/`javap` (the bundled JRE has no compiler); `javap -c` on
  `libs/languagetool-core.jar` is the authority when action semantics are unclear (LT ships no
  `.java`).
- **Oracle tooling in-repo**: `oracle/AnalyzedOracle.java` (disambiguation, modes `raw`/`dis`,
  isolates `XmlRuleDisambiguator`; see `oracle/README.md`). Rust-side harness bins in
  `crates/analysis`: `analyze` (raw/disambig dump), `grammar-examples` (self `<example>` oracle),
  `check-text` (full pipeline → `matches[]`, diff against `/v2/check`), `spell-text` (spelling only →
  `offset length suggestions`, diff against `/v2/check` `MORFOLOGIK_RULE_EN_US`); `crates/morfologik`
  `spell-check` (per-word `is_misspelled`, for membership diffing).
- **Extracted dicts** live in the session scratchpad; **regenerate** in a new session: extract
  `english.dict`/`.info` from `libs/english-pos-dict.jar`, copy `en/added.txt` + `en/removed.txt`
  beside it, and `segment.srx` from `libs/languagetool-core.jar`. For the **synthesizer** also
  extract `english_synth.dict`/`.info` + `english_tags.txt` from `libs/english-pos-dict.jar` and pass
  them as the 4th/5th args to `grammar-examples` (and 6th/7th to `check-text`). For **spelling**
  extract `en/hunspell/en_US.dict`/`.info` from `libs/english-pos-dict.jar` and copy the loose lists
  `en/hunspell/{ignore,spelling,spelling_en-US,prohibit}.txt` + `en/multiwords.txt`; `spell-text`
  takes `<english.dict> <disambiguation.xml> <segment.srx> <en_US.dict> <ignore/spelling…>
  <multiwords.txt> <prohibit.txt>`. (`TagOracle.java`/`SentOracle.java` patterns are in
  `docs/phase1-english.md` if the earlier oracles need recreating.)

## App integration contract (Phase 4 target, unchanged)

Renderer hits `localhost:8010`: `GET /v2/languages` (checks `response.ok` + echoed
`access-control-allow-origin`) and `POST /v2/check` (form: `text`, `language`, `level=default`,
`disabledRules`, `disabledCategories`; reads only `matches[].{offset,length,message,
replacements[].value,rule.id,rule.category.id}`). Config lives in Scriptorium
`languagetool-linter.ts` (disabled sets) and `languagetool.ts` (spawn).
