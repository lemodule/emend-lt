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

/// Which token an `<exception>` is tested against (LT `scope`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Current,
    Previous,
    Next,
}

/// `<and>` (all children must match the position) / `<or>` (any child) group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupKind {
    And,
    Or,
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
    /// Scope of this matcher when it is used as an `<exception>` (default
    /// `Current`); ignored for ordinary tokens.
    pub scope: Scope,
    /// When `Some`, this is an `<and>`/`<or>` group: the position must satisfy
    /// all / any of `group` (and `string`/`pos`/`exceptions` above are unused).
    pub group_kind: Option<GroupKind>,
    pub group: Vec<TokenMatcher>,
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
            scope: Scope::Current,
            group_kind: None,
            group: Vec::new(),
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
            .filter(|e| e.scope == Scope::Current)
            .any(|e| e.is_matched(r, whitespace_before))
    }

    /// Whether this token keeps reading `r` (matches it and no current-scope
    /// exception blocks it) — the per-reading predicate `filterall` needs.
    /// (Scoped exceptions are not consulted here; `filterall` skips grouped/
    /// scoped tokens.)
    pub fn keeps_reading(&self, r: &AnalyzedToken, whitespace_before: bool) -> bool {
        self.is_matched(r, whitespace_before) && !self.exception_matched(r, whitespace_before)
    }

    /// Is this an `<and>`/`<or>` group element?
    pub fn is_group(&self) -> bool {
        self.group_kind.is_some()
    }

    /// Whole-token match at position `idx`, with neighbour context so scoped
    /// exceptions (`scope="previous"|"next"`) and `<and>`/`<or>` groups work.
    pub fn matches_context(&self, tokens: &[AnalyzedTokenReadings], idx: usize) -> bool {
        if let Some(kind) = self.group_kind {
            return match kind {
                GroupKind::And => self.group.iter().all(|g| g.matches_context(tokens, idx)),
                GroupKind::Or => self.group.iter().any(|g| g.matches_context(tokens, idx)),
            };
        }
        let atr = &tokens[idx];
        let matched = atr
            .readings
            .iter()
            .any(|r| self.is_matched(r, atr.whitespace_before));
        matched && !self.any_exception_matches(tokens, idx)
    }

    /// Any `<exception>` (in its own scope) blocks the match at `idx`.
    fn any_exception_matches(&self, tokens: &[AnalyzedTokenReadings], idx: usize) -> bool {
        self.exceptions.iter().any(|e| {
            let target = match e.scope {
                Scope::Current => Some(idx),
                Scope::Previous => idx.checked_sub(1),
                Scope::Next => (idx + 1 < tokens.len()).then(|| idx + 1),
            };
            match target {
                Some(ti) => {
                    let t = &tokens[ti];
                    t.readings
                        .iter()
                        .any(|r| e.is_matched(r, t.whitespace_before))
                }
                None => false,
            }
        })
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
