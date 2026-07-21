//! Morfologik **synthesizer**: the inverse of the POS tagger. Given a lemma and
//! a POS tag it produces the surface (inflected) form(s), reading LT's
//! `english_synth.dict` (+ the `english_tags.txt` universe of tags).
//!
//! The synth `.dict` is an ordinary CFSA2 dictionary whose *keys* are
//! `lemma + "|" + postag` and whose SUFFIX-encoded value is the surface form —
//! so the existing [`Dictionary::lookup`](crate::Dictionary::lookup) already
//! decodes it (the decoded `stem` field is the surface, the `tag` field is
//! empty). This layer wraps that with LT's `BaseSynthesizer` semantics: a plain
//! tag looks up one key; a regexp tag iterates the tag universe.

use crate::dict::Dictionary;

/// LT `BaseSynthesizer` for a single language dictionary.
pub struct Synthesizer {
    dict: Dictionary,
    /// The `possibleTags` universe (from `*_tags.txt`), used when a target POS
    /// tag is a regular expression (`postag_regexp="yes"`).
    tags: Vec<String>,
}

impl Synthesizer {
    /// Load from the `english_synth.dict` bytes, its `.info` text, and the
    /// `english_tags.txt` list (one tag per line; `#` comment lines ignored).
    pub fn load(
        dict_bytes: &[u8],
        info_text: &str,
        tags_text: &str,
    ) -> Result<Synthesizer, crate::fsa::FsaError> {
        let dict = Dictionary::load(dict_bytes, info_text)?;
        let tags = tags_text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(str::to_string)
            .collect();
        Ok(Synthesizer { dict, tags })
    }

    /// The tag universe (`possibleTags`).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Synthesize the surface form(s) for `lemma` with the concrete POS tag
    /// `postag` (LT `BaseSynthesizer.lookup`: key = `lemma|postag`). Order is the
    /// dictionary's; deduplication/sorting is the caller's concern.
    pub fn synthesize(&self, lemma: &str, postag: &str) -> Vec<String> {
        let key = format!("{lemma}|{postag}");
        self.dict.lookup(&key).into_iter().map(|e| e.stem).collect()
    }

    /// Synthesize for every tag in the universe accepted by `matches`
    /// (LT `synthesizeForPosTags`), pooling the surface forms. This is the
    /// `postag_regexp="yes"` path where the target tag is itself a pattern.
    pub fn synthesize_for_tags(&self, lemma: &str, matches: impl Fn(&str) -> bool) -> Vec<String> {
        let mut out = Vec::new();
        for tag in &self.tags {
            if matches(tag) {
                out.extend(self.synthesize(lemma, tag));
            }
        }
        out
    }
}
