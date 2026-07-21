//! Phase 1.5 spelling: `MORFOLOGIK_RULE_EN_US` over the analyzed sentence.
//!
//! LT's `MorfologikSpellerRule` walks the tokens and flags each unknown word.
//! "Unknown" is the byte-exact `Speller::is_misspelled` (dictionary membership +
//! case fallback) plus LT's `SpellingCheckRule` acceptance layer: skip
//! punctuation/number/URL/e-mail tokens; treat the `ignore.txt`/`spelling*.txt`
//! word lists as additional known words; always flag `prohibit.txt` words; and
//! accept a hyphenated compound whose every part is itself a known word.
//!
//! Suggestions are a best-effort edit-distance-1 search over the dictionary
//! ranked by the stored word frequency — **not** the full Morfologik speller
//! (bounded FSA-Levenshtein with replacement pairs), so their ordering/coverage
//! is approximate. Detection (the `matches[]` offsets) is the parity target.

use std::collections::HashSet;

use matcher::AnalyzedTokenReadings;
use morfologik::Speller;

use crate::grammar::GrammarMatch;

pub struct SpellRule {
    speller: Speller,
    /// `ignore.txt` + `spelling*.txt`: words accepted as correct (never flagged,
    /// but — like LT — still usable to validate hyphen parts).
    ignore: HashSet<String>,
    /// `prohibit.txt`: words always flagged even if in the dictionary.
    prohibit: HashSet<String>,
}

impl SpellRule {
    /// Build from the `en_US.dict` bytes/info and the raw text of the word-list
    /// files (any of which may be empty). `ignore_texts` are concatenated
    /// `ignore.txt` + `spelling*.txt` bodies; `multiwords_text` is
    /// `multiwords.txt` (each capitalized term-word becomes spelling-ignored, per
    /// its header); `prohibit_text` is `prohibit.txt`.
    pub fn load(
        dict_bytes: &[u8],
        info_text: &str,
        ignore_texts: &[&str],
        multiwords_text: &str,
        prohibit_text: &str,
    ) -> Result<SpellRule, morfologik::FsaError> {
        let speller = Speller::load(dict_bytes, info_text)?;
        let mut ignore = HashSet::new();
        for t in ignore_texts {
            add_word_list(t, &mut ignore);
        }
        add_multiwords(multiwords_text, &mut ignore);
        let mut prohibit = HashSet::new();
        add_word_list(prohibit_text, &mut prohibit);
        Ok(SpellRule {
            speller,
            ignore,
            prohibit,
        })
    }

    /// Flag every misspelled token in one analyzed sentence. `base` is the
    /// sentence's character offset within the whole text.
    pub fn check_sentence(
        &self,
        tokens: &[AnalyzedTokenReadings],
        sentence_text: &str,
        base: usize,
    ) -> Vec<GrammarMatch> {
        let mut out = Vec::new();
        for tok in tokens {
            if tok.is_sent_start || tok.is_sent_end || tok.token.is_empty() {
                continue;
            }
            let w = &tok.token;
            if !is_candidate(w) || self.is_ignored(w) {
                continue;
            }
            let flagged = if self.prohibit.contains(w) {
                true
            } else if !self.speller.is_misspelled(w) {
                false
            } else {
                // A hyphenated compound is accepted if every part is known.
                !(w.contains('-') && self.hyphen_parts_known(w))
            };
            if !flagged {
                continue;
            }
            let start_byte = tok.start_pos;
            let end_byte = start_byte + w.len();
            if end_byte > sentence_text.len() {
                continue;
            }
            let offset = base + sentence_text[..start_byte].chars().count();
            let length = w.chars().count();
            out.push(GrammarMatch {
                offset,
                length,
                message: "Possible spelling mistake found.".to_string(),
                replacements: self.suggest(w),
                rule_id: "MORFOLOGIK_RULE_EN_US".to_string(),
                category_id: "TYPOS".to_string(),
            });
        }
        out
    }

    /// A word is ignored if it (or its lowercase) is in the accept list.
    fn is_ignored(&self, w: &str) -> bool {
        self.ignore.contains(w) || self.ignore.contains(&w.to_lowercase())
    }

    /// Whether a word counts as "known" for hyphen-part validation.
    fn is_known(&self, w: &str) -> bool {
        !w.is_empty() && (self.is_ignored(w) || !self.speller.is_misspelled(w))
    }

    /// Every `-`-separated part is a known word (LT hyphen-compound acceptance).
    fn hyphen_parts_known(&self, w: &str) -> bool {
        let parts: Vec<&str> = w.split('-').collect();
        parts.len() > 1 && parts.iter().all(|p| self.is_known(p))
    }

    /// Best-effort edit-distance suggestions over the dictionary, ranked by the
    /// stored frequency (approximate; see module docs). Tries distance 1 first,
    /// falling back to a bounded distance-2 search when distance 1 finds nothing.
    fn suggest(&self, w: &str) -> Vec<String> {
        let mut found: Vec<(i32, String)> = Vec::new();
        let mut seen = HashSet::new();
        let mut collect = |cand: &str, found: &mut Vec<(i32, String)>, seen: &mut HashSet<String>| {
            if cand != w && !seen.contains(cand) && self.speller.is_in_dictionary(cand) {
                seen.insert(cand.to_string());
                found.push((self.speller.frequency(cand), cand.to_string()));
            }
        };
        let e1 = edits1(w);
        for c in &e1 {
            collect(c, &mut found, &mut seen);
        }
        if found.is_empty() {
            // Distance-2, bounded so a long garbage token cannot blow up.
            let mut budget = 400_000usize;
            'outer: for c1 in &e1 {
                for c2 in edits1(c1) {
                    collect(&c2, &mut found, &mut seen);
                    budget -= 1;
                    if budget == 0 || found.len() >= 12 {
                        break 'outer;
                    }
                }
            }
        }
        found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        found.into_iter().take(8).map(|(_, c)| c).collect()
    }
}

/// Parse an `ignore.txt`/`spelling.txt`/`prohibit.txt` body: one entry per line,
/// skipping blanks and `#` comments; only single-token entries are kept (phrases
/// are handled by other LT mechanisms we do not implement here).
fn add_word_list(text: &str, set: &mut HashSet<String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Some lines carry a trailing tab-separated comment; take the first field.
        let word = line.split('\t').next().unwrap_or(line).trim();
        if !word.is_empty() && !word.contains(char::is_whitespace) {
            set.insert(word.to_string());
        }
    }
}

/// Parse `multiwords.txt` (`term<TAB>POS # comment`). LT ignores spelling for
/// every word of each term (in capitalized/all-uppercase form), so add each
/// uppercase-initial term-word to the accept set — this is where proper-noun
/// components like `Django` (from "Django Unchained") come from.
fn add_multiwords(text: &str, set: &mut HashSet<String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let term = line.split('\t').next().unwrap_or(line).trim();
        for word in term.split_whitespace() {
            if word.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                set.insert(word.to_string());
            }
        }
    }
}

/// A token is a spelling candidate if it contains a letter and is not a
/// URL/e-mail/path (LT `SpellingCheckRule` skips these).
fn is_candidate(w: &str) -> bool {
    if !w.chars().any(char::is_alphabetic) {
        return false;
    }
    !is_url_or_email(w)
}

/// LT skips URLs, e-mail addresses, and social-media handles. Approximates
/// `SpellingCheckRule`: a scheme (`://`), a `www.`/`ftp.` prefix, an `@` anywhere
/// (LT's loose e-mail/mention test skips `@mention` and `cats@dogs` alike), or a
/// `#`/`$` "cashtag/hashtag" prefix.
fn is_url_or_email(w: &str) -> bool {
    if w.contains('@') {
        return true;
    }
    if w.starts_with('#') || w.starts_with('$') {
        return true;
    }
    let lw = w.to_lowercase();
    lw.contains("://") || lw.starts_with("www.") || lw.starts_with("ftp.") || lw.starts_with("mailto:")
}

/// All strings at Damerau-less edit distance 1 from `w` (deletions,
/// transpositions, replacements, insertions) over an ASCII-letter alphabet plus
/// the letters already present (covers accented inputs minimally).
fn edits1(w: &str) -> Vec<String> {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    let mut alphabet: Vec<char> = ('a'..='z').collect();
    // Include the word's own (possibly capitalized/accented) letters.
    for &c in &chars {
        if !alphabet.contains(&c) {
            alphabet.push(c);
        }
    }
    // Also the capitalized variant of each ascii letter, so a capitalized input
    // can reach capitalized dictionary entries.
    if chars.first().map(|c| c.is_uppercase()).unwrap_or(false) {
        for c in 'A'..='Z' {
            if !alphabet.contains(&c) {
                alphabet.push(c);
            }
        }
    }
    let mut out = Vec::new();
    // deletions
    for i in 0..n {
        let mut s: String = chars[..i].iter().collect();
        s.extend(&chars[i + 1..]);
        out.push(s);
    }
    // transpositions
    for i in 0..n.saturating_sub(1) {
        let mut c = chars.clone();
        c.swap(i, i + 1);
        out.push(c.into_iter().collect());
    }
    // replacements
    for i in 0..n {
        for &a in &alphabet {
            if a == chars[i] {
                continue;
            }
            let mut c = chars.clone();
            c[i] = a;
            out.push(c.into_iter().collect());
        }
    }
    // insertions
    for i in 0..=n {
        for &a in &alphabet {
            let mut s: String = chars[..i].iter().collect();
            s.push(a);
            s.extend(&chars[i..]);
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_email_detection() {
        assert!(is_url_or_email("http://example.com"));
        assert!(is_url_or_email("www.example.com"));
        assert!(is_url_or_email("hello@world.com"));
        assert!(!is_url_or_email("co-worker"));
        assert!(!is_url_or_email("normal"));
    }

    #[test]
    fn candidate_filter() {
        assert!(is_candidate("hello"));
        assert!(!is_candidate("123"));
        assert!(!is_candidate("..."));
        assert!(!is_candidate("http://x.com"));
    }
}
