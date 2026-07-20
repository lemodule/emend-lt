//! English POS tagger, a port of LanguageTool's `EnglishTagger` (which extends
//! `BaseTagger`), built on the verified Morfologik reader (`crates/morfologik`).
//!
//! `EnglishTagger` constructs `BaseTagger("/en/english.dict", Locale.ENGLISH,
//! tagLowercaseWithUppercase=false, internTags=true)` and overrides `tag()` with
//! typographic-apostrophe handling, case-variant fallbacks, and an `in' -> ing`
//! fallback. That override is reproduced here per word.

mod casing;

use morfologik::{DictEntry, Dictionary};
use std::collections::HashMap;

/// One POS reading: base form (lemma) + tag.
pub type Reading = (String, String);

pub struct EnglishTagger {
    dict: Dictionary,
    /// Manual tag additions (`en/added.txt`), keyed by full form.
    added: HashMap<String, Vec<Reading>>,
    /// Manual tag removals (`en/removed.txt`), keyed by full form.
    removed: HashMap<String, Vec<Reading>>,
}

impl EnglishTagger {
    /// Load from raw `english.dict` bytes + `english.info` text, without manual
    /// tag additions/removals.
    pub fn load(dict_bytes: &[u8], info_text: &str) -> Result<EnglishTagger, morfologik::FsaError> {
        Self::load_with_manual(dict_bytes, info_text, "", "")
    }

    /// Load with `added.txt`/`removed.txt` overlays. LT's word tagger is a
    /// `CombiningTagger` over the FSA dictionary plus these manual lists — the
    /// effective readings are `(dict ∪ added) − removed` (no dedup: a form in
    /// both the dict and `added.txt` yields the reading twice).
    pub fn load_with_manual(
        dict_bytes: &[u8],
        info_text: &str,
        added_txt: &str,
        removed_txt: &str,
    ) -> Result<EnglishTagger, morfologik::FsaError> {
        Ok(EnglishTagger {
            dict: Dictionary::load(dict_bytes, info_text)?,
            added: parse_manual(added_txt),
            removed: parse_manual(removed_txt),
        })
    }

    /// The `WordTagger.tag` step: FSA readings plus `added.txt`, minus `removed.txt`.
    fn lookup(&self, word: &str) -> Vec<Reading> {
        let mut readings: Vec<Reading> = self
            .dict
            .lookup(word)
            .into_iter()
            .map(|DictEntry { stem, tag }| (stem, tag))
            .collect();
        if let Some(extra) = self.added.get(word) {
            readings.extend(extra.iter().cloned());
        }
        if let Some(remove) = self.removed.get(word) {
            for r in remove {
                readings.retain(|x| x != r);
            }
        }
        readings
    }

    /// All POS readings for `word`, applying `EnglishTagger.tag`'s per-word logic.
    /// An empty result means the word is untagged (LT would emit a null reading).
    pub fn tag_word(&self, word: &str) -> Vec<Reading> {
        // Typewriter-apostrophe hack: fold U+2019 to U+0027 for words len > 1.
        let word: String = if word.chars().count() > 1 && word.contains('\u{2019}') {
            word.replace('\u{2019}', "'")
        } else {
            word.to_string()
        };

        let lower_word = word.to_lowercase();
        let is_lowercase = word == lower_word;
        let is_mixed_case = casing::is_mixed_case(&word);
        let is_all_upper = casing::is_all_uppercase(&word);

        let mut l: Vec<Reading> = Vec::new();

        // normal case
        l.extend(self.lookup(&word));
        // non-lowercase, non-mixed-case words also get lowercase tags
        if !is_lowercase && !is_mixed_case {
            l.extend(self.lookup(&lower_word));
        }
        // all-uppercase proper nouns (FRANCE -> France)
        if l.is_empty() && is_all_upper {
            let first_upper = casing::uppercase_first_char(&lower_word);
            l.extend(self.lookup(&first_upper));
        }
        // "doin'" -> "doing"
        if l.is_empty() && lower_word.ends_with("in'") {
            let corrected = replace_last_char(&word, if is_all_upper { 'G' } else { 'g' });
            l.extend(self.lookup(&corrected));
            if !is_lowercase && !is_mixed_case {
                l.extend(self.lookup(&corrected.to_lowercase()));
            }
        }
        l
    }

    /// Whether the dictionary recognises `word` (any reading) — the predicate the
    /// word tokenizer needs.
    pub fn is_tagged(&self, word: &str) -> bool {
        !self.tag_word(word).is_empty()
    }
}

/// Parse a manual tagger file (`added.txt`/`removed.txt`): tab-separated
/// `fullform<TAB>lemma<TAB>postag` lines, skipping comments and blanks.
fn parse_manual(text: &str) -> HashMap<String, Vec<Reading>> {
    let mut map: HashMap<String, Vec<Reading>> = HashMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split('\t');
        match (it.next(), it.next(), it.next()) {
            (Some(form), Some(lemma), Some(pos)) if !form.is_empty() => {
                map.entry(form.to_string())
                    .or_default()
                    .push((lemma.to_string(), pos.to_string()));
            }
            _ => {}
        }
    }
    map
}

/// Replace the final character of `s` with `repl`.
fn replace_last_char(s: &str, repl: char) -> String {
    let mut chars: Vec<char> = s.chars().collect();
    if let Some(last) = chars.last_mut() {
        *last = repl;
    }
    chars.into_iter().collect()
}

/// Adapter so the tokenizer's `WordTagger` trait can be backed by this tagger.
impl tokenizer::WordTagger for &EnglishTagger {
    fn is_tagged(&self, word: &str) -> bool {
        EnglishTagger::is_tagged(self, word)
    }
}
