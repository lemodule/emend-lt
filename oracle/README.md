# Java LT oracle — disambiguation (1.3b)

`AnalyzedOracle.java` dumps the analyzed sentence, isolating the XML rule
disambiguator (`getRawAnalyzedSentence` = tagging only, then
`XmlRuleDisambiguator`), so it matches exactly what `crates/analysis` targets.
The English multiword chunker (`multiwords.txt`) is deliberately **not** applied.

## Build & run

```sh
LT=~/repositories/scriptorium/packages/desktop/resources/languagetool/lt
CP="$LT:$LT/languagetool-server.jar:$LT/libs/*"
javac -cp "$CP" AnalyzedOracle.java
# mode = raw (tagging only) | dis (+ disambiguation); probes = one sentence per line
java -cp "$CP:." AnalyzedOracle dis probes.txt > oracle.txt
```

Rust side (same output format):

```sh
# extract english.dict/.info from libs/english-pos-dict.jar and copy en/added.txt,
# en/removed.txt beside it; grab segment.srx from languagetool-core.jar.
cargo run --release -p analysis --bin analyze -- \
  dis path/to/english.dict path/to/disambiguation.xml probes.txt path/to/segment.srx > rust.txt
```

Compare `TOK|` lines (align by the `===|` input blocks; a small structural
comparator isolates disambiguation diffs from any residual seg/tok diffs).
