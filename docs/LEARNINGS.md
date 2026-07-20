# What we learned reverse-engineering LanguageTool 6.6

A reference of the concrete, load-bearing facts discovered while reimplementing LanguageTool's
English engine in Rust (`emend-lt`). Everything here was verified against **bundled Java
LanguageTool 6.6** (in Scriptorium), not inferred from docs. It is meant to save the next
person from re-deriving it. Companion docs: `phase1-english.md` (progress), `NEXT.md` (tasks),
`phase0-scope.md` (target rules).

---

## 0. The big picture

LanguageTool is **~10% engine, ~90% data**. Reimplementing it means writing a new *interpreter*
for LT's data formats, not translating Java line by line. The pipeline for one check request:

```
text
  └─ SRX sentence segmentation            (segment.srx)          → sentences
       └─ word tokenization               (EnglishWordTokenizer) → tokens   ← calls the tagger!
            └─ POS tagging                 (english.dict + added/removed) → readings
                 └─ disambiguation         (disambiguation.xml)  → pruned readings
                      └─ pattern rules     (grammar.xml)         → matches[]
                      └─ hand-coded rules  (Java classes)        → matches[]
```

Data lives in: Morfologik FSA `.dict` files, SRX XML, disambiguation XML, grammar XML, and a
handful of plain-text lists. The HTTP surface an embedding app needs is tiny: `GET /v2/languages`
(readiness + CORS echo) and `POST /v2/check`. See `NEXT.md` for the exact request/response contract.

---

## 1. Where the English data actually lives (bundle layout)

In `…/resources/languagetool/lt/`:

- Compiled LT **classes are loose files** under `lt/org/languagetool/…` (e.g.
  `language/English.class`, `tokenizers/en/EnglishWordTokenizer.class`) — *not* only in jars. The
  app's classpath is `lt : languagetool-server.jar : libs/*`.
- **English FSA dictionaries are inside `libs/english-pos-dict.jar`**, not extracted to disk:
  `org/languagetool/resource/en/english.dict` (POS), `.../hunspell/en_US.dict` (spelling), plus
  `.info` siblings. **Extract them** to work with them. (Relevant to Phase 4 bundling.)
- Text resources on disk: `resource/en/added.txt`, `removed.txt`, `hunspell/spelling*.txt`,
  `disambiguation.xml`; rules at `rules/en/grammar.xml`.
- SRX: `segment.srx` is inside `libs/languagetool-core.jar` at `org/languagetool/resource/`.
- Morfologik CLI tools (great oracle): `libs/morfologik-tools.jar`
  (`morfologik.tools.Launcher fsa_decompile …`).

---

## 2. Morfologik `.dict` — the FSA format (CFSA2)

- Magic `\fsa` (`5c 66 73 61`) then a **version byte `0xC6` = CFSA2** (compressed FSA v2). This is
  **not** the `fst` crate's format — the roadmap's assumption was wrong. Port the reader from the
  reference `morfologik.fsa.CFSA2` Java class.
- Header after magic+version: `u16` big-endian flags, `u8` label-map size, the label map, then
  the arc bytes. Our English dicts have flags `0x0007` (FLEXIBLE|STOPBIT|NEXTBIT); NUMBERS not set.
- Arc byte: low 5 bits = index into label map (0 ⇒ label byte is stored inline next); bits
  `0x80` = target-is-next, `0x40` = last-arc, `0x20` = final-arc. Targets are vints. Full decode
  in `crates/morfologik/src/fsa.rs` (verified byte-exact against `fsa_decompile`).
- **Dictionary layout on top of the FSA** (from the `.info`):
  - `fsa.dict.separator=+`, `fsa.dict.encoder=SUFFIX`, `fsa.dict.encoding=utf-8`.
  - **POS (stemming) dict**: each accepted sequence is `inflected + SEP + encodedStem + SEP + tag`
    (e.g. `walked+C+VBD`).
  - **SUFFIX decoding**: the first byte of `encodedStem` is `('A' + N)`; drop `N` trailing bytes
    from the inflected form, then append the rest. `walked` + `C`(N=2) → `walk`; `mice` + `Emouse`
    → `mouse`; `better` + `Ggood` → `good`.
  - **Spelling dict**: `fsa.dict.frequency-included=true`; entries are `word + SEP + freqByte`
    (e.g. `house+A`). Membership = a `SEP` arc exists out of the node the word spells to.

---

## 3. SRX sentence segmentation (`segment.srx`)

- Header has `cascade="yes"`. `<languagemap>`s are matched (Java `matches()` = full match) against
  the language code **in document order**; with cascade, **all** matches contribute. For `en-*`
  the effective, ordered rule list is **GeneralImportant → English → Default** (94 rules).
- Each `<rule break="yes|no">` has a `<beforebreak>` and `<afterbreak>` Java regex
  (`useJavaRegex="yes"`). At a candidate position, the **first rule in order** whose beforebreak
  matches text ending there *and* afterbreak matches text starting there decides break/no-break.
- **Implementation trick that works**: compile each rule to a zero-width assertion
  `(?<=beforebreak)(?=afterbreak)` and run it over the **full text** with `fancy-regex`
  (variable-length lookbehind supported). No slicing, so `\b`/lookarounds see real neighbours.
  First rule to claim a position wins. Verified identical to LT over 40 tricky probes.
- **Java→Rust regex translation** is essentially just `\uXXXX` → `\x{XXXX}`. `\p{Lu}`, `(?i)`,
  `(?!…)`, `\b` all carry over. `fancy-regex` is required (standard `regex` lacks lookaround).
- Sentences **include trailing whitespace** (the break is placed after it); concatenating
  segments reproduces the input.

---

## 4. Word tokenization (`EnglishWordTokenizer`)

- The English tokenizer does **not** use the base `StringTokenizer`; it splits with the regex
  `[wordChars]+|[^wordChars]` (word-char runs, or one non-word char). `wordChars` includes letters,
  digits, `-`, `$`, combining marks, etc. — but **not** `.` `/` `:` space.
- Apostrophes are **shielded** before the split (`'`→`xxAPOSTYPEWxx`, `'`→`xxAPOSTYPOGxx`) so
  contractions stay inside a word run, then restored, then split by four contraction regexes
  (`n't`, `'s`, `'re`, `'ll`, `'ve`, `'d`, `'m`, `'t was/were/is`).
- **`wordsToAdd` calls the POS tagger.** A token containing `-`/`'` is kept whole iff
  `EnglishTagger.isTagged(...)` recognises it, else split on apostrophes. This is why
  `don't → [do, n't]` (n't is a tagged form) and `o'clock` stays one token. **⇒ correct word
  tokenization is downstream of the tagger.** We inject the tagger via a `WordTagger` trait.
- **URL/email re-joining** (`joinEMails`, `joinUrls`) merges tokens back together afterwards.
  **Gotcha:** LT's `URL_CHARS = [a-zA-Z…0-9/%$-_.+!*'(),?#~]` contains the **range `$`–`_`**
  (U+0024–U+005F), which covers `:` `;` `=` `?` `@`. Escaping the hyphen (making `$` `-` `_`
  literals) silently breaks `https://`/`ftp://` joining while `www.` still works. The differential
  harness caught exactly this on 2 probes.
- Whitespace and punctuation are returned as their **own tokens**; concatenation reproduces input.

---

## 5. POS tagging (`EnglishTagger` / `BaseTagger`)

- `EnglishTagger` = `BaseTagger("/en/english.dict", Locale.ENGLISH, tagLowercaseWithUppercase=false,
  internTags=true)` with an overridden per-word `tag()`:
  1. If len>1 and contains `'`(U+2019): fold to `'`, mark typographic-apostrophe.
  2. Look up the exact word. Then, if not-lowercase and not-mixed-case, also look up the lowercase.
  3. If still empty and all-uppercase: look up `uppercaseFirstChar(lower)` (proper nouns, `FRANCE`).
  4. If still empty and `lower` ends with `in'`: retry with `in' → ing`/`inG`.
  5. If still empty: one null (untagged) reading.
- **The word tagger is not the bare FSA.** LT overlays manual lists via a CombiningTagger:
  effective readings = **(dict ∪ `added.txt`) − `removed.txt`**, and it **does not dedup** — a form
  present in both the dict and `added.txt` yields the reading twice (e.g. `climbdowns` → two
  identical `climbdown/NNS`). `emphasis` shows `NN:UN` only because `removed.txt` strips the dict's
  `NN`. Files are TSV `fullform<TAB>lemma<TAB>postag`.
- `StringTools` case predicates that drive branching: `isAllUppercase` (no lowercase letter),
  `isCapitalizedWord` (first upper, rest no-uppercase-letter), `isMixedCase =
  !allUpper && !capitalized && hasSomeUppercaseLetter`.
- Verified byte-identical to LT over 3,025 words (dictionary forms + case variants + OOV).

---

## 6. Token-pattern rules (shared by `disambiguation.xml` and `grammar.xml`)

Both formats express rules as a `<pattern>` of `<token>`s over the analyzed sentence, so the
matcher is built once (`crates/matcher`) and shared.

- **Single `<token>` match** (`PatternToken.isMatched`):
  `hasStringTest ? (textMatches XOR negate) && (posMatches XOR negate_pos)
                 : !negate && (posMatches XOR negate_pos)`, gated by optional `spacebefore`.
  - String test uses the surface form, or the **lemma when `inflected`**; regexp is a full match;
    case-insensitive unless `case_sensitive`.
  - POS: `postag` exact or `postag_regexp`; special `UNKNOWN` matches a reading with no tag.
  - A token matches a position iff **some reading matches and no `<exception>` matches**.
- **Sequence**: quantifiers `min` (0 ⇒ optional), `max` (`unlimited` ⇒ -1), `skip` (gap allowed
  before the next token; -1 ⇒ unlimited). `<marker>` delimits the highlighted/target sub-span; its
  bounds must be mapped through the *actual* matched token path (skips shift positions).
- **`<!ENTITY>` macros**: the rule files define regex fragments in their prolog (`<!ENTITY
  uncommon_verbs "…">`, used 169×; `&apostrophe;`, `&months;`, …) and reference them as `&name;`
  inside token text/attrs. quick-xml does **not** expand these — the whole file must be
  pre-expanded first (96 macros in grammar, 11 in disambiguation). Without it, ~600 grammar
  patterns silently keep literal `&name;` text.
- **Feature usage in the real English files** (drives what to build):
  - grammar.xml: `postag` 14180, `regexp` 12225, `postag_regexp` 5932, `min` 4722, `chunk_re` 2670,
    `inflected` 2044, `chunk` 2028, `skip` 1718, `case_sensitive` 1353, `spacebefore` 864,
    `max` 532, `negate` 25, `negate_pos` 13; `<marker>` 14447, `<match>` 1896, `<and>` 51.
  - disambiguation.xml actions: `replace` 358, `filter` 80, `remove` 75, `add` 75, `filterall` 25,
    `unify` 2.
- **Coverage today** (core features, `pattern-coverage` bin): grammar **85.2%** (4724/5543),
  disambiguation **91.7%** (975/1063) of patterns fully supported (`<and>`/`<or>` groups + scoped
  exceptions now supported). The single remaining gap, flagged per-pattern (never silently wrong):
  **`chunk`/`chunk_re`** (818 grammar / 86 disambig — needs the OpenNLP phrase chunker, a separate
  subsystem). Disambiguation token-parity vs Java LT is **89.6%**; the whole gap is the chunker
  (chunk-reading rules + cascades where a supported rule sees readings a chunk rule should have
  narrowed first).

---

## 7. Verification methodology (why this is trustworthy)

Every component is diffed against a Java-LT oracle, per the roadmap's differential-testing plan.
The **ground-truth oracles** used so far:

- FSA reader ↔ `morfologik.tools.Launcher fsa_decompile` (byte-exact on all sequences).
- Segmentation ↔ `lang.getSentenceTokenizer().tokenize()` (a small `SentOracle.java`).
- Word tokenization ↔ `lang.getWordTokenizer().tokenize()` (same oracle).
- Tagging ↔ `EnglishTagger.INSTANCE.tag()` (`TagOracle.java`).
- Disambiguation ↔ `getRawAnalyzedSentence()` + `XmlRuleDisambiguator.disambiguate()`
  (`AnalyzedOracle.java`, modes `raw`/`dis`). Raw byte-identical; disambiguation ≈85.9% token
  parity, gap = deferred matcher features, ~zero false positives. When the exact action semantics
  were unclear, we **decompiled `DisambiguationPatternRuleReplacer` with `javap -c`** — the
  authoritative source, since LT ships no `.java`.
- (Next) grammar ↔ `POST /v2/check`.

This has already paid off: it caught the `URL_CHARS` range bug and the `added.txt`/`removed.txt`
overlay, each as a handful of concrete mismatches rather than silent drift. **Rule of thumb: build
the oracle first, diff a tricky corpus, and treat any diff as a defect to close.**

---

## 8. Corrections to the original roadmap

1. `.dict` is Morfologik **CFSA2**, not an `fst`-crate format — a real reader had to be written.
2. The **word tokenizer depends on the tagger**, so 1.2 and 1.3 interlock (not strictly sequential).
3. The tagger includes **`added.txt`/`removed.txt`** overlays, not just the FSA.
4. Rule files need **`<!ENTITY>` expansion** before parsing.
5. Disambiguation and grammar **share one token-pattern matcher** — worth building once up front.
6. `chunk`-based rules (a large chunk of grammar.xml) need the **OpenNLP chunker**, a distinct
   subsystem that can be deferred (Phase 0's fired-rule set skews away from chunk-heavy style rules).
