//! Raw analysis + disambiguation pipeline.
//!
//! Turns a single (already segmented) sentence string into the
//! `Vec<AnalyzedTokenReadings>` that `crates/matcher` runs over — the Rust
//! counterpart of LanguageTool's `JLanguageTool.getRawAnalyzedSentence`, then
//! optionally `XmlRuleDisambiguator.disambiguate`.
//!
//! Raw analysis mirrors LT: word-tokenize, POS-tag each token, drop whitespace
//! tokens (recording `whitespace_before` on the following token), prepend a
//! `SENT_START` token, and append a `SENT_END` reading to the last token.

use matcher::{AnalyzedToken, AnalyzedTokenReadings};
use tagger::EnglishTagger;
use tokenizer::EnglishWordTokenizer;

pub mod disambig;
pub mod grammar;
pub mod spelling;

pub use disambig::{parse_disambig_rules, DisambigRule};
pub use grammar::{check_sentence, parse_grammar_rules, GrammarMatch, GrammarRule};
pub use spelling::SpellRule;

/// Owns the tagger and tokenizer, producing analyzed sentences.
pub struct Analyzer<'a> {
    tokenizer: EnglishWordTokenizer<&'a EnglishTagger>,
    tagger: &'a EnglishTagger,
}

impl<'a> Analyzer<'a> {
    pub fn new(tagger: &'a EnglishTagger) -> Self {
        Analyzer {
            tokenizer: EnglishWordTokenizer::new(tagger),
            tagger,
        }
    }

    /// LT `getRawAnalyzedSentence` for one sentence: tag-only, no disambiguation.
    pub fn raw(&self, sentence: &str) -> Vec<AnalyzedTokenReadings> {
        let raw_tokens = self.tokenizer.tokenize(sentence);

        let mut out: Vec<AnalyzedTokenReadings> = vec![AnalyzedTokenReadings::sent_start()];
        let mut pos = 0usize; // byte offset within the sentence
        let mut whitespace_before = false;
        for tok in &raw_tokens {
            if is_whitespace(tok) {
                whitespace_before = true;
                pos += tok.len();
                continue;
            }
            let readings = self.tag(tok);
            out.push(AnalyzedTokenReadings::word(tok, readings, whitespace_before, pos));
            whitespace_before = false;
            pos += tok.len();
        }

        // Append SENT_END to the last real token (LT `AnalyzedTokenReadings.setSentEnd`).
        if let Some(last) = out.last_mut() {
            if !last.is_sent_start {
                set_sent_end(last);
            }
        }
        out
    }

    /// Tag one surface token into readings, using an untagged `(None, None)`
    /// reading when the tagger recognises nothing (LT's null reading).
    fn tag(&self, surface: &str) -> Vec<AnalyzedToken> {
        let readings = self.tagger.tag_word(surface);
        if readings.is_empty() {
            vec![AnalyzedToken::new(surface, None, None)]
        } else {
            readings
                .into_iter()
                .map(|(lemma, pos)| AnalyzedToken::new(surface, Some(&lemma), Some(&pos)))
                .collect()
        }
    }
}

/// LT `setSentEnd`: append a `SENT_END` reading. If the token was untagged (a
/// single null reading), that null reading is dropped first (LT `addReading`).
fn set_sent_end(t: &mut AnalyzedTokenReadings) {
    // SENT_END reading copies the lemma of the token's last existing reading
    // (null for an untagged token, whose lone null reading is then dropped).
    let lemma = t.readings.last().and_then(|r| r.lemma.clone());
    if t.readings.len() == 1 && t.readings[0].has_no_tag() {
        t.readings.clear();
    }
    t.readings.push(AnalyzedToken::new(
        &t.token,
        lemma.as_deref(),
        Some("SENT_END"),
    ));
    t.is_sent_end = true;
}

fn is_whitespace(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| c.is_whitespace())
}
