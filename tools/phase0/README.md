# Phase 0 tooling — target-scope enumeration

Enumerates which LanguageTool rules actually fire under the consuming app's request config,
so Phase 1 targets the real subset instead of LT's full rule base. See the generated
[`docs/phase0-scope.md`](../../docs/phase0-scope.md) for the output.

## Prerequisites

A running LanguageTool 6.6 server. The bundled one from Scriptorium works:

```sh
LT=~/repositories/scriptorium/packages/desktop/resources/languagetool
"$LT/jre-darwin-arm64/bin/java" -Xms256m -Xmx1024m \
  -cp "$LT/lt:$LT/lt/languagetool-server.jar:$LT/lt/libs/*" \
  org.languagetool.server.HTTPServer --port 8099 \
  --allow-origin "app://emend" --cacheSize 2000
```

## Pipeline

```sh
# 1. fire the corpus through the oracle with the app's exact disabled filters
python3 enumerate_rules.py --server http://localhost:8099 --corpus corpus.json --out scope.json

# 2. tag each fired rule xml vs no-grammar.xml (reads the bundled LT rule data)
python3 classify_rules.py            # --rules-root defaults to the Scriptorium bundle

# 3. render the human-readable scope doc (sub-classifies the no-XML bucket)
python3 build_scope_doc.py           # writes ../../docs/phase0-scope.md
```

## Files

- `corpus.json` — per-language probe sentences. **Grow this** as Phase 3 differential
  testing finds rules the seed corpus never triggered.
- `enumerate_rules.py` — mirrors the app config (`level=default`, the app's
  `disabledRules`/`disabledCategories`) and collects fired `rule.id`/`category.id` with
  hit-frequency; also asserts the disabled filter didn't leak.
- `classify_rules.py` — XML-pattern (`id="…"` present in a `grammar.xml`/`.xml`) vs
  no-data-file.
- `build_scope_doc.py` — renders the doc, splitting no-XML rules into spelling/Morfologik,
  data-driven replace, and genuine hand-coded Java tiers.

`scope.json` / `scope-classified.json` are regenerated artifacts (git-ignored).
