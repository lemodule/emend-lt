#!/usr/bin/env python3
"""Phase 0 — classify each fired rule as XML-pattern vs hand-coded.

For every rule id in scope.json, search the bundled LanguageTool rule data for that
language: if `id="RID"` appears in any .xml rule file, the rule is XML-pattern (Phase 1.4
work); otherwise it is implemented by a Java class with no data file (Phase 1.5 work).

Writes scope-classified.json.
"""
import argparse
import json
import os
import re
import subprocess

# app language code -> LT rules subdirectory (variant collapses to the base dir)
LANG_DIR = {
    "en-US": "en", "en-GB": "en", "en-AU": "en", "en-CA": "en",
    "fr": "fr", "de-DE": "de", "de-AT": "de", "de-CH": "de",
    "es": "es", "pt-PT": "pt", "pt-BR": "pt", "it": "it", "pl": "pl",
}


def is_xml_rule(rules_root, subdir, rule_id):
    """True if `id="rule_id"` occurs in any .xml under the language's rules dir."""
    target = os.path.join(rules_root, subdir)
    if not os.path.isdir(target):
        return False
    # ripgrep-free: grep is on every mac; fall back to os.walk if unavailable.
    pat = f'id="{rule_id}"'
    try:
        r = subprocess.run(
            ["grep", "-rlF", pat, "--include=*.xml", target],
            capture_output=True, text=True,
        )
        return r.returncode == 0 and bool(r.stdout.strip())
    except FileNotFoundError:
        rx = re.compile(re.escape(pat))
        for dirpath, _, files in os.walk(target):
            for fn in files:
                if fn.endswith(".xml"):
                    with open(os.path.join(dirpath, fn), errors="ignore") as f:
                        if rx.search(f.read()):
                            return True
        return False


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--scope", default="scope.json")
    ap.add_argument(
        "--rules-root",
        default=os.path.expanduser(
            "~/repositories/scriptorium/packages/desktop/resources/languagetool/lt/"
            "org/languagetool/rules"
        ),
    )
    ap.add_argument("--out", default="scope-classified.json")
    args = ap.parse_args()

    with open(args.scope) as f:
        scope = json.load(f)

    for lang, r in scope["languages"].items():
        subdir = LANG_DIR.get(lang, lang)
        for rule in r["rules"]:
            rule["kind"] = (
                "xml" if is_xml_rule(args.rules_root, subdir, rule["id"])
                else "hand-coded"
            )
        r["xml_count"] = sum(1 for x in r["rules"] if x["kind"] == "xml")
        r["hand_coded_count"] = sum(1 for x in r["rules"] if x["kind"] == "hand-coded")

    with open(args.out, "w") as f:
        json.dump(scope, f, indent=2, ensure_ascii=False)
    print(f"Wrote {args.out}")
    for lang, r in scope["languages"].items():
        print(f"  {lang:7s}  xml={r['xml_count']:2d}  hand-coded={r['hand_coded_count']:2d}")


if __name__ == "__main__":
    main()
