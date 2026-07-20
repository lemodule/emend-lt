//! Shared token-pattern matcher for LanguageTool's `disambiguation.xml` and
//! `grammar.xml`. Both file formats express rules as a `<pattern>` of `<token>`
//! elements over an analyzed (tokenized + POS-tagged) sentence; this crate is
//! that common matching core, built once and driven by the disambiguator (1.3b)
//! and the grammar rule engine (1.4).

pub mod analyzed;
pub mod entities;
pub mod parse;
pub mod pattern;
pub mod token;

pub use analyzed::{AnalyzedToken, AnalyzedTokenReadings};
pub use entities::{expand, parse_entity_defs};
pub use parse::{parse_pattern, ParsedPattern};
pub use pattern::{Pattern, PatternMatch};
pub use token::{GroupKind, PosMatcher, Scope, StringMatcher, TokenMatcher};

#[cfg(test)]
mod integration_tests {
    use super::*;

    fn sent(words: &[(&str, &str, &str)]) -> Vec<AnalyzedTokenReadings> {
        words
            .iter()
            .enumerate()
            .map(|(i, (w, l, p))| {
                AnalyzedTokenReadings::word(
                    w,
                    vec![AnalyzedToken::new(w, Some(l), Some(p))],
                    i > 0,
                    i,
                )
            })
            .collect()
    }

    #[test]
    fn parses_and_matches_a_realistic_pattern() {
        // "a/an" + optional adjective + singular noun, marking the article.
        let xml = r#"
          <pattern>
            <marker><token regexp="yes">a|an</token></marker>
            <token postag="JJ" min="0"/>
            <token postag="NN"/>
          </pattern>"#;
        let parsed = parse_pattern(xml, false).unwrap();
        assert!(parsed.unsupported.is_empty());
        let p = &parsed.pattern;

        let s = sent(&[("a", "a", "DT"), ("big", "big", "JJ"), ("cat", "cat", "NN")]);
        let m = p.match_at(&s, 0).unwrap();
        assert_eq!((m.from_token, m.to_token), (0, 3));
        assert_eq!((m.marker_from_token, m.marker_to_token), (0, 1));

        // Also matches without the optional adjective.
        let s2 = sent(&[("an", "an", "DT"), ("apple", "apple", "NN")]);
        assert!(p.match_at(&s2, 0).is_some());

        // No noun -> no match.
        let s3 = sent(&[("a", "a", "DT"), ("big", "big", "JJ")]);
        assert!(p.match_at(&s3, 0).is_none());
    }

    #[test]
    fn exception_in_pattern_parses() {
        let xml = r#"
          <pattern>
            <token postag="NN.*" postag_regexp="yes"><exception>cat</exception></token>
          </pattern>"#;
        let parsed = parse_pattern(xml, false).unwrap();
        let p = &parsed.pattern;
        assert!(p.match_at(&sent(&[("dog", "dog", "NN")]), 0).is_some());
        assert!(p.match_at(&sent(&[("cat", "cat", "NN")]), 0).is_none());
    }

    #[test]
    fn flags_unsupported_chunk() {
        let xml = r#"<pattern><token chunk="B-NP"/></pattern>"#;
        let parsed = parse_pattern(xml, false).unwrap();
        assert!(parsed.unsupported.contains(&"chunk".to_string()));
    }
}
