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

### Word tokenizer — implemented; full verification gated on 1.3

`crates/tokenizer` — a port of LT's `EnglishWordTokenizer`: the `[wordChars]+|[^wordChars]`
split, the apostrophe-shielding dance, the four contraction patterns (`n't`, `'s`, `'re`, …),
leading/trailing-hyphen handling, and the base `joinEMails`/`joinUrls` re-merging.

**Second finding — a real coupling:** LT's word tokenizer **calls the POS tagger**. Inside
`wordsToAdd`, a token containing `-`/`'` is kept whole iff `EnglishTagger.isTagged(...)`
recognises it — this is why `don't → [do, n't]` keeps `n't` whole, and `o'clock` stays one
token. So byte-exact word tokenization is intrinsically **downstream of Phase 1.3** (the
tagger), which itself builds on the 1.1 FSA reader.

The tagger is therefore injected as a `WordTagger` trait. The tokenizer is complete now; unit
tests with a stub tagger confirm the mechanism (`don't→[do, n't]`, URL/email joining,
punctuation/space splitting, trailing hyphen). The full-corpus oracle diff runs once the real
tagger (1.3) is wired in.

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

## Next (English)

- **1.3 POS tagger + disambiguator** — uses the 1.1 reader (`english.dict`) +
  `en/disambiguation.xml`. Unblocks byte-exact word tokenization.
- 1.4 XML pattern-rule engine (`en/grammar.xml`) — the long pole.
- 1.5 hand-coded English rules (`EN_CONTRACTION_SPELLING`, `EN_A_VS_AN`, …).
