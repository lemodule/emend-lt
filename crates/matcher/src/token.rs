//! A single `<token>` matcher, ported from LanguageTool's `PatternToken`.
//!
//! `isMatched(reading)` (per LT):
//! ```text
//! if has string test:  textMatches XOR negate  AND  posMatches XOR negate_pos
//! else:                !negate                  AND  posMatches XOR negate_pos
//! ```
//! plus an optional `spacebefore` (whitespace) gate. A token as a whole matches a
//! position iff some reading `isMatched` and no exception matches.

use crate::analyzed::{AnalyzedToken, AnalyzedTokenReadings};
use fancy_regex::Regex;

/// String test on a token's surface form (or lemma, when `inflected`).
pub enum StringMatcher {
    Literal { text: String, case_sensitive: bool },
    Regex(Regex),
}

impl StringMatcher {
    pub fn literal(text: &str, case_sensitive: bool) -> Self {
        StringMatcher::Literal {
            text: text.to_string(),
            case_sensitive,
        }
    }

    /// A full-match regex (LT uses `Matcher.matches()`), case-insensitive unless
    /// `case_sensitive`.
    pub fn regex(pattern: &str, case_sensitive: bool) -> Result<Self, String> {
        let prefix = if case_sensitive { "" } else { "(?i)" };
        let full = format!("{prefix}(?s)^(?:{pattern})$");
        Regex::new(&full).map(StringMatcher::Regex).map_err(|e| e.to_string())
    }

    pub fn matches(&self, s: &str) -> bool {
        match self {
            StringMatcher::Literal { text, case_sensitive } => {
                if *case_sensitive {
                    text == s
                } else {
                    text.eq_ignore_ascii_case(s) || text.to_lowercase() == s.to_lowercase()
                }
            }
            StringMatcher::Regex(re) => re.is_match(s).unwrap_or(false),
        }
    }
}

/// POS test on a reading's tag.
pub enum PosMatcher {
    Exact(String),
    Regex(Regex),
    /// LT's special `UNKNOWN` postag: matches a reading with no tag.
    Unknown,
}

impl PosMatcher {
    pub fn new(postag: &str, is_regex: bool) -> Result<Self, String> {
        if postag == "UNKNOWN" {
            return Ok(PosMatcher::Unknown);
        }
        if is_regex {
            Regex::new(&format!("(?s)^(?:{postag})$"))
                .map(PosMatcher::Regex)
                .map_err(|e| e.to_string())
        } else {
            Ok(PosMatcher::Exact(postag.to_string()))
        }
    }

    pub fn matches_reading(&self, token: &AnalyzedToken) -> bool {
        match self {
            PosMatcher::Unknown => token.has_no_tag(),
            _ => match &token.pos {
                None => false,
                Some(tp) => match self {
                    PosMatcher::Exact(p) => p == tp,
                    PosMatcher::Regex(re) => re.is_match(tp).unwrap_or(false),
                    PosMatcher::Unknown => unreachable!(),
                },
            },
        }
    }
}

pub struct TokenMatcher {
    pub string: Option<StringMatcher>,
    pub inflected: bool,
    pub negate: bool,
    pub pos: Option<PosMatcher>,
    pub negate_pos: bool,
    /// `Some(true)` requires whitespace before; `Some(false)` requires none.
    pub spacebefore: Option<bool>,
    pub exceptions: Vec<TokenMatcher>,
    // Quantifiers (sequence-level, read by the pattern matcher).
    pub min: i32,
    pub max: i32,
    pub skip: i32,
}

impl Default for TokenMatcher {
    fn default() -> Self {
        TokenMatcher {
            string: None,
            inflected: false,
            negate: false,
            pos: None,
            negate_pos: false,
            spacebefore: None,
            exceptions: Vec::new(),
            min: 1,
            max: 1,
            skip: 0,
        }
    }
}

impl TokenMatcher {
    fn test_token<'a>(&self, r: &'a AnalyzedToken) -> &'a str {
        if self.inflected {
            r.lemma.as_deref().unwrap_or(&r.token)
        } else {
            &r.token
        }
    }

    fn pos_matched(&self, r: &AnalyzedToken) -> bool {
        match &self.pos {
            None => true,
            Some(p) => p.matches_reading(r),
        }
    }

    /// LT `PatternToken.isMatched` for one reading (exceptions handled separately).
    pub fn is_matched(&self, r: &AnalyzedToken, whitespace_before: bool) -> bool {
        if let Some(want) = self.spacebefore {
            if whitespace_before != want {
                return false;
            }
        }
        let pos_ok = self.pos_matched(r) ^ self.negate_pos;
        match &self.string {
            Some(sm) => (sm.matches(self.test_token(r)) ^ self.negate) && pos_ok,
            None => !self.negate && pos_ok,
        }
    }

    fn exception_matched(&self, r: &AnalyzedToken, whitespace_before: bool) -> bool {
        self.exceptions
            .iter()
            .any(|e| e.is_matched(r, whitespace_before))
    }

    /// Whether this token keeps reading `r` (matches it and no exception blocks
    /// it) — the per-reading predicate the `filterall` disambig action needs.
    pub fn keeps_reading(&self, r: &AnalyzedToken, whitespace_before: bool) -> bool {
        self.is_matched(r, whitespace_before) && !self.exception_matched(r, whitespace_before)
    }

    /// Whole-token match at a position: some reading matches and no
    /// (current-scope) exception matches.
    pub fn matches_position(&self, atr: &AnalyzedTokenReadings) -> bool {
        let mut matched = false;
        let mut excepted = false;
        for r in &atr.readings {
            if self.is_matched(r, atr.whitespace_before) {
                matched = true;
            }
            if self.exception_matched(r, atr.whitespace_before) {
                excepted = true;
            }
        }
        matched && !excepted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzed::AnalyzedTokenReadings;

    fn tok(word: &str, readings: &[(&str, &str)]) -> AnalyzedTokenReadings {
        let rs = readings
            .iter()
            .map(|(l, p)| AnalyzedToken::new(word, Some(l), Some(p)))
            .collect();
        AnalyzedTokenReadings::word(word, rs, true, 0)
    }

    #[test]
    fn literal_case_insensitive_by_default() {
        let mut m = TokenMatcher::default();
        m.string = Some(StringMatcher::literal("the", false));
        assert!(m.matches_position(&tok("The", &[("the", "DT")])));
        m.string = Some(StringMatcher::literal("the", true));
        assert!(!m.matches_position(&tok("The", &[("the", "DT")])));
    }

    #[test]
    fn postag_regex_and_negate() {
        let mut m = TokenMatcher::default();
        m.pos = Some(PosMatcher::new("VB.*", true).unwrap());
        assert!(m.matches_position(&tok("run", &[("run", "VBP")])));
        assert!(!m.matches_position(&tok("cat", &[("cat", "NN")])));
        m.negate_pos = true;
        assert!(m.matches_position(&tok("cat", &[("cat", "NN")])));
    }

    #[test]
    fn inflected_matches_lemma() {
        let mut m = TokenMatcher::default();
        m.inflected = true;
        m.string = Some(StringMatcher::literal("be", false));
        assert!(m.matches_position(&tok("were", &[("be", "VBD")])));
    }

    #[test]
    fn exception_blocks_match() {
        let mut m = TokenMatcher::default();
        m.pos = Some(PosMatcher::new("NN.*", true).unwrap());
        let mut exc = TokenMatcher::default();
        exc.string = Some(StringMatcher::literal("cat", false));
        m.exceptions.push(exc);
        assert!(!m.matches_position(&tok("cat", &[("cat", "NN")])));
        assert!(m.matches_position(&tok("dog", &[("dog", "NN")])));
    }

    #[test]
    fn empty_token_matches_anything() {
        let m = TokenMatcher::default();
        assert!(m.matches_position(&tok("whatever", &[("whatever", "NN")])));
    }
}
