//! SRX sentence segmenter compatible with LanguageTool's `segment.srx`.
//!
//! Algorithm (SRX / loomchild semantics): each rule becomes a zero-width
//! assertion `(?<=beforebreak)(?=afterbreak)` evaluated against the *full* text
//! (so `\b` and lookarounds see real neighbours — no slicing). For every byte
//! offset where some rule fires, the first rule in list order decides whether a
//! sentence break occurs there. Segments are the substrings between breaks;
//! trailing whitespace stays attached to the preceding sentence, matching LT.

mod regex_translate;
pub mod srx;

use fancy_regex::Regex;
use regex_translate::java_to_fancy;
use srx::{SrxDocument, SrxRule};

struct CompiledRule {
    is_break: bool,
    assertion: Regex,
}

pub struct Segmenter {
    rules: Vec<CompiledRule>,
}

impl Segmenter {
    /// Build a segmenter for `lang_code` (e.g. `"en-US"`) from raw `segment.srx`.
    pub fn from_srx(xml: &str, lang_code: &str) -> Result<Segmenter, String> {
        let doc = SrxDocument::parse(xml)?;
        let rules = doc.rules_for(lang_code);
        Ok(Segmenter::from_rules(&rules))
    }

    fn from_rules(rules: &[SrxRule]) -> Segmenter {
        let mut compiled = Vec::with_capacity(rules.len());
        for r in rules {
            let before = java_to_fancy(&r.before);
            let after = java_to_fancy(&r.after);
            // Empty before/after => an always-true assertion on that side.
            let lb = if before.is_empty() {
                String::new()
            } else {
                format!("(?<=(?:{before}))")
            };
            let la = if after.is_empty() {
                String::new()
            } else {
                format!("(?=(?:{after}))")
            };
            let pat = format!("{lb}{la}");
            // A rule with both sides empty would match everywhere and is useless;
            // skip it. Also skip rules that fail to compile (rare TM-markup rules
            // in the Default set that never fire on prose), so one bad pattern
            // doesn't sink the whole rule list.
            if pat.is_empty() {
                continue;
            }
            match Regex::new(&pat) {
                Ok(assertion) => compiled.push(CompiledRule {
                    is_break: r.is_break,
                    assertion,
                }),
                Err(e) => {
                    eprintln!("segmenter: skipping uncompilable SRX rule ({e}): {pat}");
                }
            }
        }
        Segmenter { rules: compiled }
    }

    /// Split `text` into sentences. Concatenating the result reproduces `text`.
    pub fn segment(&self, text: &str) -> Vec<String> {
        let breaks = self.break_offsets(text);
        let mut out = Vec::with_capacity(breaks.len() + 1);
        let mut prev = 0;
        for &b in &breaks {
            if b > prev {
                out.push(text[prev..b].to_string());
                prev = b;
            }
        }
        if prev < text.len() {
            out.push(text[prev..].to_string());
        }
        out
    }

    /// Byte offsets at which a sentence break occurs (sorted, unique, interior).
    fn break_offsets(&self, text: &str) -> Vec<usize> {
        // First rule (in order) that fires at a position decides it.
        // decided[pos] = Some(is_break) once claimed.
        use std::collections::BTreeMap;
        let mut decided: BTreeMap<usize, bool> = BTreeMap::new();

        for rule in &self.rules {
            let mut it = rule.assertion.find_iter(text);
            while let Some(Ok(m)) = it.next() {
                let pos = m.start(); // zero-width: start == end
                if pos == 0 || pos >= text.len() {
                    continue; // no break before the first or after the last char
                }
                decided.entry(pos).or_insert(rule.is_break);
            }
        }

        decided
            .into_iter()
            .filter_map(|(pos, is_break)| if is_break { Some(pos) } else { None })
            .collect()
    }
}
