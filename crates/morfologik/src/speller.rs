//! Morfologik **speller** membership core: LT's misspelling decision for a single
//! word, reading `en_US.dict` (+ its `.info`).
//!
//! This is the byte-exact port of `morfologik.speller.Speller.isMisspelled`
//! (delegated to by `MorfologikSpeller.isMisspelled`). It answers *"is this word
//! unknown?"* — the detection half of `MORFOLOGIK_RULE_EN_US`. Suggestion
//! generation (the FSA-guided Levenshtein search) is a separate, much larger
//! algorithm and is only approximated elsewhere.
//!
//! For `en_US` the dictionary is frequency-included and case-converting: a word
//! is misspelled iff it is not in the dictionary **and** — unless it is
//! mixed-case — its lowercase form is not in the dictionary either. Words
//! containing a digit are ignored (LT `ignore-numbers`).

use crate::dict::Dictionary;

pub struct Speller {
    dict: Dictionary,
}

impl Speller {
    /// Load from the `en_US.dict` bytes + its `.info` text.
    pub fn load(dict_bytes: &[u8], info_text: &str) -> Result<Speller, crate::fsa::FsaError> {
        Ok(Speller {
            dict: Dictionary::load(dict_bytes, info_text)?,
        })
    }

    /// Dictionary membership (LT `Speller.isInDictionary`): the word spells a
    /// complete entry (frequency tail present).
    pub fn is_in_dictionary(&self, word: &str) -> bool {
        self.dict.contains(word)
    }

    /// Stored word frequency (LT `Speller.getFrequency`), for ranking suggestions.
    pub fn frequency(&self, word: &str) -> i32 {
        self.dict.frequency(word)
    }

    /// LT `Speller.isMisspelled` for a case-converting, number-ignoring dictionary
    /// (`en_US`): not in the dictionary, and — unless mixed-case — its lowercase
    /// is also unknown. A word containing a digit is never misspelled.
    pub fn is_misspelled(&self, word: &str) -> bool {
        if word.is_empty() || !contains_no_digit(word) {
            return false;
        }
        if self.is_in_dictionary(word) {
            return false;
        }
        // convertsCase: a capitalized/all-caps word can match a lowercase entry,
        // but a mixed-case word (iPhone-shape) is only ever matched verbatim.
        is_mixed_case(word) || !self.is_in_dictionary(&word.to_lowercase())
    }
}

/// LT `Speller.containsNoDigit`: no character is a decimal digit.
fn contains_no_digit(s: &str) -> bool {
    !s.chars().any(|c| c.is_ascii_digit() || c.is_numeric())
}

/// LT `Speller.isAllUppercase`: no letter is lowercase (non-letters ignored).
fn is_all_uppercase(s: &str) -> bool {
    !s.chars().any(|c| c.is_alphabetic() && c.is_lowercase())
}

/// LT `Speller.isNotAllLowercase`: some letter is uppercase.
fn is_not_all_lowercase(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic() && !c.is_lowercase())
}

/// LT `Speller.isNotCapitalizedWord`: NOT (first char uppercase and every later
/// letter lowercase). Empty / lowercase-initial strings count as "not
/// capitalized" (returns `true`).
fn is_not_capitalized_word(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return true;
    };
    if !first.is_uppercase() {
        return true;
    }
    chars.any(|c| c.is_alphabetic() && !c.is_lowercase())
}

/// LT `Speller.isMixedCase`: not all-uppercase, not a capitalized word, and not
/// all-lowercase (e.g. `tHe`, `McDonald`).
fn is_mixed_case(s: &str) -> bool {
    !is_all_uppercase(s) && is_not_capitalized_word(s) && is_not_all_lowercase(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_case_classification() {
        assert!(is_mixed_case("tHe"));
        assert!(is_mixed_case("McDonald"));
        assert!(is_mixed_case("iPhone"));
        assert!(!is_mixed_case("Paris"));
        assert!(!is_mixed_case("hello"));
        assert!(!is_mixed_case("HELLO"));
        assert!(!is_mixed_case("The"));
    }

    #[test]
    fn digit_guard() {
        assert!(!contains_no_digit("covid19"));
        assert!(contains_no_digit("covid"));
    }
}
