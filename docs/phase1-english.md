# Phase 1 — English engine progress

English-first, built bottom-up in the roadmap's dependency order and verified against
Java LanguageTool 6.6 at each step.

## 1.1 — Morfologik FSA reader ✅ done & verified

`crates/morfologik` — a from-scratch Rust reader for LanguageTool's `.dict` format, backing
both spelling (`MORFOLOGIK_RULE_EN_US`) and POS tagging.

**Key finding correcting the roadmap:** LT's `.dict` files are Morfologik **CFSA2** (magic
`\fsa`, version `0xC6`), *not* a format the `fst` crate reads. The reader was ported directly
from the reference `morfologik.fsa.CFSA2` Java class.

**Verified** byte-exact against Morfologik's own Java decompiler (`fsa_decompile`):

| Dict | Sequences | Rust == Java |
|---|---|---|
| `english.dict` (POS) | 259,353 | ✅ byte-exact |
| `en_US.dict` (spelling) | 206,845 | ✅ byte-exact |

Plus: spelling membership (`becuase`/`occured` → false), SUFFIX POS decode
(`walked→walk`, `children→child`, `mice→mouse`, `better→good/well`), locked in unit tests.

## 1.2 — Tokenizer + SRX sentence splitter

### Sentence splitter ✅ done & verified byte-exact

`crates/segmenter` — an SRX segmenter reproducing LT's `segment.srx`.

- Parses SRX (`quick-xml`), resolves the **cascade** of `<languagemap>`s for a code — for
  `en-*` that is `GeneralImportant → English → Default` (94 rules) concatenated in order.
- Each rule compiles to a zero-width assertion `(?<=beforebreak)(?=afterbreak)` evaluated
  on the **full text** via `fancy-regex` (variable-length lookbehind), so `\b`/lookarounds
  see real neighbours — no slicing artifacts. First rule to fire at a position wins.
- Java→Rust regex translation is just `\uXXXX` → `\x{XXXX}`; everything else (`\p{Lu}`,
  `(?i)`, `(?!…)`) carries over.

**Verified** identical to LT's `SRXSentenceTokenizer` over **40 tricky probes** (abbreviations
`Dr.`/`p.m.`/`U.S.A.`/`Ph.D.`/`No. 5`/`vol.`, ellipses, nested quotes/brackets, tabs, multiple
spaces): **46/46 and 40/40 sentences byte-identical, zero diffs.**

### Word tokenizer ✅ done & verified byte-exact (with the 1.3 tagger wired in)

`crates/tokenizer` — a port of LT's `EnglishWordTokenizer`: the `[wordChars]+|[^wordChars]`
split, the apostrophe-shielding dance, the four contraction patterns (`n't`, `'s`, `'re`, …),
leading/trailing-hyphen handling, and the base `joinEMails`/`joinUrls` re-merging.

**Second finding — a real coupling:** LT's word tokenizer **calls the POS tagger**. Inside
`wordsToAdd`, a token containing `-`/`'` is kept whole iff `EnglishTagger.isTagged(...)`
recognises it — this is why `don't → [do, n't]` keeps `n't` whole, and `o'clock` stays one
token. The tagger is injected as a `WordTagger` trait; with the real 1.3 tagger backing it,
tokenization is verified **byte-identical to LT across 58 sentences** (three corpora:
contractions, possessives, hyphenation, URLs, emails, `o'clock`, `rock'n'roll`, …).

**Bug the differential harness caught (exactly the roadmap's point):** LT's `URL_CHARS`
pattern `[…$-_…]` is a *range* `$`–`_` (covers `:`, `;`, `=`, `?`, `@`, …). An initial
mistranslation escaped the hyphen into three literals, which silently broke joining of
`https://`/`ftp://` URLs while `www.` URLs still worked. The oracle diff flagged it on exactly
two probes; fixed in `url_chars()`.

## Reproduce

Java oracle (bundled LT 6.6 classes) + Rust, over probe files:

```sh
# sentence segmentation parity (expects "IDENTICAL")
javac -cp "<lt>:<lt>/languagetool-server.jar:<lt>/libs/*" SentOracle.java
java  -cp "<lt>:...:oracle_classes" SentOracle en-US probes.txt | grep -E '^===|^SENT' > oracle.txt
cargo run --release -p segmenter --bin sent-split -- segment.srx en-US probes.txt > rust.txt
diff oracle.txt rust.txt

cargo test        # morfologik (3) + segmenter (3) + tokenizer (4)
```

## 1.3 — POS tagger + disambiguator

### POS tagger ✅ done & verified byte-exact

`crates/tagger` — a port of LT's `EnglishTagger` (extends `BaseTagger`) on the 1.1 reader:
per-word case-variant fallbacks (exact → lowercase → all-caps-proper-noun), the `in'→ing`
fallback, and typographic-apostrophe folding.

**Third finding:** the word tagger is not the bare FSA — LT wraps it in a `CombiningTagger`
that overlays `en/added.txt` (manual additions) and subtracts `en/removed.txt`. Effective
readings are `(dict ∪ added) − removed`, and it does **not** dedup (a form in both the dict
and `added.txt` yields the reading twice, e.g. `climbdowns`). This was discovered by two
mismatches (`emphasis`, `climbdowns`) in the first parity run and then resolved.

**Verified byte-identical to LT's `EnglishTagger` over 3,025 words** (3,000 random inflected
forms from the dictionary + case variants, contractions, and OOV words): zero diffs on the
full `lemma/POS` reading multiset per word.

### Disambiguator (1.3b) ✅ done — raw byte-exact, disambiguation ≈85.9% token parity

`crates/analysis` turns text into the `Vec<AnalyzedTokenReadings>` the matcher runs over and then
applies `en/disambiguation.xml` as an ordered rewrite system.

- **Raw analysis** (`Analyzer::raw`, LT `getRawAnalyzedSentence`): word-tokenize → POS-tag → drop
  whitespace (recording `whitespace_before`) → prepend `SENT_START` → append a `SENT_END` reading
  to the last token. **Byte-identical** to Java LT on the probe set.
- **Rule engine** (`disambig.rs`): parses `<rule>`/`<rulegroup>` = `<pattern>` (compiled by the
  shared matcher) + `<antipattern>`s + `<disambig>`. Actions: `replace`, `add`, `remove`, `filter`,
  `filterall`; `unify`/`ignore_spelling`/`immunize` are reading-no-ops (they don't touch lemma/POS
  in LT). Applied in document order.

**Oracle**: `oracle/AnalyzedOracle.java` runs `getRawAnalyzedSentence` then **only**
`XmlRuleDisambiguator` (not the hybrid — the multiword chunker over `multiwords.txt` is a separate
subsystem, deliberately excluded), dumping `surface|lemma/POS#…` per token in modes `raw`/`dis`.
`crates/analysis/src/bin/analyze.rs` emits the identical format; a structural comparator aligns by
sentence and compares readings.

**Result** over a 1,500-sentence corpus (grammar.xml examples): raw byte-identical; post-
disambiguation **token-reading parity ≈ 85.9%**. Every remaining diff is a *false negative* from a
deferred matcher feature (chunk-based rules, `<and>`/`<or>`, `<exception scope=>`) or a cascade of
one — **~zero genuine false positives** (the discipline: under-apply, never mis-apply).

**Action semantics that bit us (all confirmed against the oracle / decompiled
`DisambiguationPatternRuleReplacer`):**

1. **`&quot;` etc. are separate `GeneralRef` events** in quick-xml — `parse.rs` ignored them, so
   every `<token>&quot;</token>` silently became an empty (match-anything) token and its
   `<disambig>` corrupted the first real token of *every* sentence. Now resolved in `parse.rs`.
2. **Bare `<disambig postag="X"/>` (REPLACE)** touches **only the first marker token** (`fromPos`)
   and collapses it to a *single* reading `(surface, X, lemma)`, where `lemma` = the **last**
   existing reading whose POS is exactly `X`, else the token's first-reading lemma (null if
   untagged). So `best[…good/JJS,well/JJS,…]+IN → best[well/JJS]` (and `of/IN` untouched);
   `are[are/NN,be/VBP] → be/VBP`; `10[] → 10[_/CD]`.
3. **`filter`/`remove` take a `postag` OR `<wd>`**: `postag` is a regex over readings (keep, resp.
   drop, matching ones); `<wd>` matches by lemma+POS (absent attr = wildcard). A `<wd>`-list
   REPLACE is a *hard* set to the wd readings.
4. **A `remove`/`filter` that empties a token leaves one null `(_/_)` reading** (LT never lets a
   token reach zero readings) — that's why LT prints `∅/∅`, not an empty token.
5. **`filterall`** keeps, per marker token, only the readings its *pattern token* matched.
6. **`<antipattern>`s** must be honored (761 of them): if one matches over the rule's span, the
   match is suppressed. A rule whose pattern **or any antipattern** uses an unsupported feature is
   skipped wholesale — otherwise we'd fire when an unsupported antipattern would have blocked
   (this was the `have/VB` bug). Antipatterns are where most of the coverage gap concentrates.
7. **`SENT_END`** reading copies the token's *last existing reading's* lemma (null when untagged,
   whose lone null reading is then dropped) — not the surface form.

## Shared token-pattern matcher (serves 1.3b disambiguation + 1.4 grammar)

`crates/matcher` — the common `<pattern>`/`<token>` matching core both rule formats compile to.

- **Data model** (`analyzed.rs`): `AnalyzedToken` (surface/lemma/POS reading),
  `AnalyzedTokenReadings` (a position with all readings + whitespace context + SENT_START).
- **Token matcher** (`token.rs`): LT's `PatternToken.isMatched` exactly — string test
  (literal/regexp, `case_sensitive`, `inflected`→lemma, `negate`), POS test
  (`postag`/`postag_regexp`, `UNKNOWN`, `negate_pos`), `spacebefore`, and `<exception>`s
  (current scope), combined with the `XOR` negation logic.
- **Sequence matcher** (`pattern.rs`): greedy backtracking over `min`/`max`/`skip`, with
  `<marker>` bounds tracked through the actual matched token path.
- **Parser** (`parse.rs`) + **entity expansion** (`entities.rs`): compiles `<pattern>` XML,
  first expanding the files' `<!ENTITY>` regex-fragment macros (96 in grammar, 11 in
  disambiguation — e.g. `&uncommon_verbs;` used 169×), which quick-xml does not do itself.

**Validated against real data** (`pattern-coverage` bin over the actual English files):

| File | Patterns | Parse OK | Fully supported by current features |
|---|---|---|---|
| `grammar.xml` | 5,543 | 5,542 | 4,697 (**84.7%**) |
| `disambiguation.xml` | 1,063 | 1,061 | 874 (**82.2%**) |

Quantified gaps (deliberately deferred, not silently wrong — flagged per pattern via
`ParsedPattern::unsupported`): **`chunk`/`chunk_re`** (818 grammar / 86 disambiguation — needs
the OpenNLP phrase chunker, a separate subsystem) and **`<and>`/`<or>` token groups** (49 /
111). 16 unit tests cover the token + sequence + parser + entity semantics.

## Next (English)

- ~~1.3b Disambiguator~~ ✅ done (`crates/analysis`).
- **`<and>`/`<or>` groups + `<exception scope=>` + OpenNLP chunker** in the matcher — these three
  features are the entire disambiguation parity gap (and most of the grammar gap).
- 1.4 XML pattern-rule engine (`en/grammar.xml`) — reuses the same matcher; the long pole.
- 1.5 hand-coded English rules (`EN_CONTRACTION_SPELLING`, `EN_A_VS_AN`, …).
- Known raw-tagger edge (not disambiguation): hyphenated plurals like `four-year-olds` get one
  fewer NNS reading than LT (LT also emits a self-lemma `four-year-olds/NNS`).
