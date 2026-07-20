//! A `<pattern>`: an ordered list of [`TokenMatcher`]s plus a `<marker>` span,
//! matched against the whitespace-free token array of a sentence.
//!
//! Matching is greedy backtracking over each token's `min`/`max` repetition and
//! `skip` gap, mirroring LT's performer closely enough for the common cases; the
//! pathological skip/max interactions are hardened against the analyzed-sentence
//! oracle when the disambiguator/grammar action layers land.

use crate::analyzed::AnalyzedTokenReadings;
use crate::token::TokenMatcher;

pub struct Pattern {
    pub tokens: Vec<TokenMatcher>,
    /// Pattern-token index of the first `<marker>`ed token (default 0).
    pub mark_from: usize,
    /// Pattern-token index just past the last `<marker>`ed token (default len).
    pub mark_to: usize,
}

/// A successful match, in sentence-token-index space (half-open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    /// First matched token (the whole pattern, not just the marker).
    pub from_token: usize,
    /// One past the last matched token.
    pub to_token: usize,
    /// Marker span (what a rule highlights / a disambig rule targets).
    pub marker_from_token: usize,
    pub marker_to_token: usize,
    /// For each pattern token (element), the sentence-token index where it began
    /// matching — resolves `\N` backreferences (`\1` = `element_start[0]`).
    pub element_start: Vec<usize>,
}

impl Pattern {
    pub fn new(tokens: Vec<TokenMatcher>) -> Self {
        let n = tokens.len();
        Pattern {
            tokens,
            mark_from: 0,
            mark_to: n,
        }
    }

    /// Find the first match at or after `start` (scanning start positions in
    /// order, as LT does), or `None`.
    pub fn find_at_or_after(
        &self,
        tokens: &[AnalyzedTokenReadings],
        start: usize,
    ) -> Option<PatternMatch> {
        for s in start..tokens.len() {
            if let Some(m) = self.match_at(tokens, s) {
                return Some(m);
            }
        }
        None
    }

    /// Try to match the whole pattern starting exactly at token `start`.
    pub fn match_at(&self, tokens: &[AnalyzedTokenReadings], start: usize) -> Option<PatternMatch> {
        if self.tokens.is_empty() {
            return None;
        }
        // rec[pi] = (start_token, count) chosen for pattern token pi.
        let mut rec = vec![(0usize, 0usize); self.tokens.len()];
        let end = self.solve(tokens, 0, start, &mut rec)?;

        let (mf, mt) = self.marker_span(&rec);
        Some(PatternMatch {
            from_token: start,
            to_token: end,
            marker_from_token: mf,
            marker_to_token: mt,
            element_start: rec.iter().map(|(s, _)| *s).collect(),
        })
    }

    /// Recursive greedy backtracking. Returns the end token index (exclusive).
    fn solve(
        &self,
        tokens: &[AnalyzedTokenReadings],
        pi: usize,
        ti: usize,
        rec: &mut Vec<(usize, usize)>,
    ) -> Option<usize> {
        if pi == self.tokens.len() {
            return Some(ti);
        }
        let pt = &self.tokens[pi];
        let len = tokens.len();
        let max_c = if pt.max < 0 { i32::MAX } else { pt.max } as usize;
        let min_c = pt.min.max(0) as usize;

        // Count consecutive matches available from ti (capped at max_c).
        let mut avail = 0usize;
        while ti + avail < len && avail < max_c && pt.matches_context(tokens, ti + avail) {
            avail += 1;
        }
        if avail < min_c {
            return None;
        }

        // Greedy: try the longest repetition first.
        let mut c = avail;
        loop {
            let after = ti + c;
            let max_skip = if pt.skip < 0 {
                len.saturating_sub(after)
            } else {
                (pt.skip as usize).min(len.saturating_sub(after))
            };
            rec[pi] = (ti, c);
            // Nearest match first: try the smallest gap.
            for g in 0..=max_skip {
                if let Some(end) = self.solve(tokens, pi + 1, after + g, rec) {
                    return Some(end);
                }
            }
            if c == min_c {
                break;
            }
            c -= 1;
        }
        None
    }

    /// Map the marker's pattern-token bounds to sentence-token indices using the
    /// recorded match path.
    fn marker_span(&self, rec: &[(usize, usize)]) -> (usize, usize) {
        let from = rec[self.mark_from].0;
        // last marked pattern token is mark_to-1
        let last = self.mark_to.saturating_sub(1).max(self.mark_from);
        let (start, count) = rec[last];
        (from, start + count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzed::{AnalyzedToken, AnalyzedTokenReadings};
    use crate::token::{PosMatcher, StringMatcher, TokenMatcher};

    fn sent(words: &[(&str, &str, &str)]) -> Vec<AnalyzedTokenReadings> {
        // (surface, lemma, pos)
        words
            .iter()
            .enumerate()
            .map(|(i, (w, l, p))| {
                AnalyzedTokenReadings::word(w, vec![AnalyzedToken::new(w, Some(l), Some(p))], i > 0, i)
            })
            .collect()
    }

    fn lit(s: &str) -> TokenMatcher {
        TokenMatcher {
            string: Some(StringMatcher::literal(s, false)),
            ..Default::default()
        }
    }
    fn pos(p: &str) -> TokenMatcher {
        TokenMatcher {
            pos: Some(PosMatcher::new(p, true).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn consecutive_match() {
        let s = sent(&[("the", "the", "DT"), ("big", "big", "JJ"), ("cat", "cat", "NN")]);
        let p = Pattern::new(vec![lit("the"), pos("JJ"), pos("NN")]);
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!((m.from_token, m.to_token), (0, 3));
    }

    #[test]
    fn no_match_returns_none() {
        let s = sent(&[("the", "the", "DT"), ("cat", "cat", "NN")]);
        let p = Pattern::new(vec![lit("the"), pos("JJ")]);
        assert!(p.match_at(&s, 0).is_none());
    }

    #[test]
    fn skip_allows_gap() {
        // "the" skip=2 then a noun: matches "the big old cat"
        let s = sent(&[
            ("the", "the", "DT"),
            ("big", "big", "JJ"),
            ("old", "old", "JJ"),
            ("cat", "cat", "NN"),
        ]);
        let mut the = lit("the");
        the.skip = 2;
        let p = Pattern::new(vec![the, pos("NN")]);
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!(m.to_token, 4);
    }

    #[test]
    fn optional_token_min0() {
        // optional adjective
        let s = sent(&[("the", "the", "DT"), ("cat", "cat", "NN")]);
        let mut adj = pos("JJ");
        adj.min = 0;
        let p = Pattern::new(vec![lit("the"), adj, pos("NN")]);
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!(m.to_token, 2);
    }

    #[test]
    fn max_repetition() {
        let s = sent(&[
            ("very", "very", "RB"),
            ("very", "very", "RB"),
            ("big", "big", "JJ"),
        ]);
        let mut rb = pos("RB");
        rb.max = -1; // unlimited
        let p = Pattern::new(vec![rb, pos("JJ")]);
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!(m.to_token, 3);
    }

    #[test]
    fn marker_span_tracks_actual_tokens() {
        let s = sent(&[("the", "the", "DT"), ("big", "big", "JJ"), ("cat", "cat", "NN")]);
        let mut p = Pattern::new(vec![lit("the"), pos("JJ"), pos("NN")]);
        p.mark_from = 1;
        p.mark_to = 2; // marks just the adjective
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!((m.marker_from_token, m.marker_to_token), (1, 2));
    }
}
