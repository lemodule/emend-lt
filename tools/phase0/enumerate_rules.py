#!/usr/bin/env python3
"""Phase 0 — enumerate which LanguageTool rules actually fire for the app's config.

Sends every corpus sentence to a running LanguageTool server using the EXACT request
parameters the Scriptorium renderer uses (level=default, and the app's disabledRules /
disabledCategories), then reports the distinct rule.id + rule.category.id that appear,
ranked by hit frequency, per language.

Usage:
    python3 enumerate_rules.py --server http://localhost:8099 \
        --corpus corpus.json --out scope.json

The output JSON is consumed by build_scope_doc.py to render docs/phase0-scope.md.
"""
import argparse
import json
import sys
import urllib.parse
import urllib.request
from collections import Counter, defaultdict

# --- The app's exact filter config (packages/frontend/.../languagetool-linter.ts) ---
LEVEL = "default"
DISABLED_CATEGORIES = ["STYLE", "REPETITIONS_STYLE"]
DISABLED_RULES = [
    "TYPOGRAPHIC_QUOTES",
    "EN_QUOTES",
    "FR_QUOTES",
    "DOUBLE_QUOTE_AFTER_COMMA",
    "ELLIPSIS",
    "UNPAIRED_BRACKETS",
    "UPPERCASE_SENTENCE_START",
]


def check(server, text, language):
    data = urllib.parse.urlencode({
        "text": text,
        "language": language,
        "level": LEVEL,
        "disabledRules": ",".join(DISABLED_RULES),
        "disabledCategories": ",".join(DISABLED_CATEGORIES),
    }).encode()
    req = urllib.request.Request(
        f"{server}/v2/check",
        data=data,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.load(resp)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--server", default="http://localhost:8099")
    ap.add_argument("--corpus", default="corpus.json")
    ap.add_argument("--out", default="scope.json")
    args = ap.parse_args()

    with open(args.corpus) as f:
        corpus = json.load(f)

    result = {}
    filter_leak = []  # rules that slipped through despite being disabled

    for lang, sentences in corpus.items():
        if lang.startswith("_"):
            continue
        rule_freq = Counter()
        rule_cat = {}
        rule_msg = {}
        cat_freq = Counter()
        sample = defaultdict(list)
        for text in sentences:
            try:
                resp = check(args.server, text, lang)
            except Exception as e:  # noqa: BLE001
                print(f"[{lang}] request failed: {e}", file=sys.stderr)
                continue
            for m in resp.get("matches", []):
                rule = m.get("rule", {})
                rid = rule.get("id", "?")
                cat = rule.get("category", {})
                cid = cat.get("id", "?")
                rule_freq[rid] += 1
                cat_freq[cid] += 1
                rule_cat[rid] = cid
                rule_msg.setdefault(rid, m.get("message", ""))
                if len(sample[rid]) < 2:
                    off, ln = m.get("offset", 0), m.get("length", 0)
                    sample[rid].append(text[off:off + ln])
                if rid in DISABLED_RULES or cid in DISABLED_CATEGORIES:
                    filter_leak.append((lang, rid, cid))
        result[lang] = {
            "total_matches": sum(rule_freq.values()),
            "distinct_rules": len(rule_freq),
            "category_frequency": dict(cat_freq.most_common()),
            "rules": [
                {
                    "id": rid,
                    "category": rule_cat[rid],
                    "hits": n,
                    "message": rule_msg[rid],
                    "samples": sample[rid],
                }
                for rid, n in rule_freq.most_common()
            ],
        }

    out = {
        "config": {
            "level": LEVEL,
            "disabledCategories": DISABLED_CATEGORIES,
            "disabledRules": DISABLED_RULES,
        },
        "filter_leak": filter_leak,
        "languages": result,
    }
    with open(args.out, "w") as f:
        json.dump(out, f, indent=2, ensure_ascii=False)

    print(f"Wrote {args.out}")
    if filter_leak:
        print(f"WARNING: {len(filter_leak)} matches leaked past the disabled filter!")
    for lang, r in result.items():
        print(f"  {lang:7s}  {r['distinct_rules']:3d} distinct rules, "
              f"{r['total_matches']:4d} matches")


if __name__ == "__main__":
    main()
