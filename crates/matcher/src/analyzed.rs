//! The analyzed-sentence data model the pattern matcher runs over.
//!
//! Mirrors LanguageTool: a token carries its surface form plus one or more POS
//! *readings* (lemma + tag). The matcher operates on the whitespace-free token
//! array, each token remembering whether whitespace preceded it. A synthetic
//! `SENT_START` token is prepended so rules can anchor to sentence start.

/// One POS reading of a token: `(lemma, tag)`. A `None` tag means "no reading"
/// (an unknown/untagged word).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzedToken {
    pub token: String,
    pub lemma: Option<String>,
    pub pos: Option<String>,
}

impl AnalyzedToken {
    pub fn new(token: &str, lemma: Option<&str>, pos: Option<&str>) -> Self {
        AnalyzedToken {
            token: token.to_string(),
            lemma: lemma.map(str::to_string),
            pos: pos.map(str::to_string),
        }
    }

    /// LT `hasNoTag`: the reading carries no POS tag.
    pub fn has_no_tag(&self) -> bool {
        self.pos.is_none()
    }
}

/// A token position with all its readings and its whitespace context.
#[derive(Debug, Clone)]
pub struct AnalyzedTokenReadings {
    pub token: String,
    pub readings: Vec<AnalyzedToken>,
    pub whitespace_before: bool,
    /// Byte offset of the token in the original sentence text.
    pub start_pos: usize,
    pub is_sent_start: bool,
    pub is_sent_end: bool,
}

impl AnalyzedTokenReadings {
    pub fn word(
        token: &str,
        readings: Vec<AnalyzedToken>,
        whitespace_before: bool,
        start_pos: usize,
    ) -> Self {
        AnalyzedTokenReadings {
            token: token.to_string(),
            readings,
            whitespace_before,
            start_pos,
            is_sent_start: false,
            is_sent_end: false,
        }
    }

    pub fn sent_start() -> Self {
        AnalyzedTokenReadings {
            token: String::new(),
            readings: vec![AnalyzedToken::new("", None, Some("SENT_START"))],
            whitespace_before: false,
            start_pos: 0,
            is_sent_start: true,
            is_sent_end: false,
        }
    }
}
