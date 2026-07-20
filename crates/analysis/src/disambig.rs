//! Parse `en/disambiguation.xml` into a list of ordered rewrite rules, and apply
//! them to an analyzed sentence — the disambiguator action layer atop
//! `crates/matcher`.
//!
//! Each `<rule>` is a `<pattern>` (compiled by the shared matcher) plus a
//! `<disambig>` describing an action on the matched tokens: `replace`, `add`,
//! `remove`, `filter`, `filterall`, `unify`, `ignore_spelling`, or `immunize`.
//! Rules run in document order (an ordered rewrite system).

use matcher::{parse_pattern, AnalyzedToken, AnalyzedTokenReadings, Pattern};
use quick_xml::events::Event;
use quick_xml::{Reader, Writer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Replace,
    Add,
    Remove,
    Filter,
    Filterall,
    Unify,
    IgnoreSpelling,
    Immunize,
}

impl Action {
    fn parse(s: Option<&str>) -> Action {
        match s {
            None => Action::Replace, // LT default action is REPLACE
            Some("add") => Action::Add,
            Some("remove") => Action::Remove,
            Some("filter") => Action::Filter,
            Some("filterall") => Action::Filterall,
            Some("unify") => Action::Unify,
            Some("ignore_spelling") => Action::IgnoreSpelling,
            Some("immunize") => Action::Immunize,
            Some("replace") | Some(_) => Action::Replace,
        }
    }
}

/// One `<wd>` reading in a `<disambig>` body: the replacement/added reading.
#[derive(Debug, Clone)]
pub struct Wd {
    pub lemma: Option<String>,
    pub pos: Option<String>,
}

pub struct DisambigRule {
    pub id: String,
    pub pattern: Pattern,
    /// `<antipattern>`s: if any matches over the rule's span, the match is
    /// suppressed (LT `getAntiPatterns`).
    pub antipatterns: Vec<Pattern>,
    pub action: Action,
    /// `<wd>` readings under `<disambig>` (position-by-position for the marker).
    pub wds: Vec<Wd>,
    /// Bare `<disambig postag="...">` — replace the marker tokens' POS.
    pub disambig_postag: Option<String>,
    /// Features this rule uses that the layer cannot yet run faithfully; when
    /// non-empty the rule is skipped rather than applied wrongly.
    pub unsupported: Vec<String>,
}

/// A captured `<pattern>` / `<antipattern>` awaiting compilation: its inner XML
/// (rewrapped as `<pattern>`) and its `case_sensitive` flag.
struct CapturedPattern {
    xml: String,
    case_sensitive: bool,
}

/// Parse a whole disambiguation XML document (already entity-expanded).
pub fn parse_disambig_rules(xml: &str) -> Result<Vec<DisambigRule>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut rules: Vec<DisambigRule> = Vec::new();

    // Per-rule accumulators.
    let mut in_rule = false;
    let mut rule_id = String::new();
    let mut in_disambig = false;
    let mut action = Action::Replace;
    let mut disambig_postag: Option<String> = None;
    let mut wds: Vec<Wd> = Vec::new();
    let mut has_match = false;
    let mut captured_pattern: Option<CapturedPattern> = None;
    let mut captured_antis: Vec<CapturedPattern> = Vec::new();

    // Active <pattern>/<antipattern> capture (inner XML rewrapped as <pattern>).
    let mut capture: Option<Writer<Vec<u8>>> = None;
    let mut capture_is_anti = false;
    let mut capture_cs = false;

    let is_wrapper = |n: &[u8]| n == b"pattern" || n == b"antipattern";

    loop {
        let ev = reader.read_event().map_err(|e| format!("XML error: {e}"))?;

        // Mirror inner events into the active capture (skip the wrapper tags
        // themselves — we emit a synthetic <pattern> wrapper instead).
        if let Some(w) = capture.as_mut() {
            let skip = match &ev {
                Event::Start(e) | Event::Empty(e) => is_wrapper(e.local_name().as_ref()),
                Event::End(e) => is_wrapper(e.local_name().as_ref()),
                _ => false,
            };
            if !skip {
                let _ = w.write_event(ev.borrow());
            }
        }

        match &ev {
            Event::Eof => break,
            Event::Start(e) => match e.local_name().as_ref() {
                b"rule" => {
                    in_rule = true;
                    rule_id = attr(e, "id").unwrap_or_default();
                }
                b"pattern" | b"antipattern" if in_rule => {
                    capture_is_anti = e.local_name().as_ref() == b"antipattern";
                    capture_cs = attr(e, "case_sensitive").as_deref() == Some("yes");
                    let mut w = Writer::new(Vec::new());
                    let _ = w.write_event(Event::Start(
                        quick_xml::events::BytesStart::new("pattern"),
                    ));
                    capture = Some(w);
                }
                b"disambig" if in_rule => {
                    in_disambig = true;
                    action = Action::parse(attr(e, "action").as_deref());
                    disambig_postag = attr(e, "postag");
                }
                b"match" if in_disambig => has_match = true,
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"disambig" if in_rule => {
                    action = Action::parse(attr(e, "action").as_deref());
                    disambig_postag = attr(e, "postag");
                }
                b"wd" if in_disambig => {
                    wds.push(Wd {
                        lemma: attr(e, "lemma"),
                        pos: attr(e, "pos"),
                    });
                }
                b"match" if in_disambig => has_match = true,
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                b"pattern" | b"antipattern" if capture.is_some() => {
                    let mut w = capture.take().unwrap();
                    let _ =
                        w.write_event(Event::End(quick_xml::events::BytesEnd::new("pattern")));
                    let xml = String::from_utf8_lossy(&w.into_inner()).into_owned();
                    let cap = CapturedPattern {
                        xml,
                        case_sensitive: capture_cs,
                    };
                    if capture_is_anti {
                        captured_antis.push(cap);
                    } else {
                        captured_pattern = Some(cap);
                    }
                }
                b"disambig" => in_disambig = false,
                b"rule" if in_rule => {
                    if let Some(rule) = build_rule(
                        std::mem::take(&mut rule_id),
                        captured_pattern.take(),
                        std::mem::take(&mut captured_antis),
                        action,
                        std::mem::take(&mut wds),
                        disambig_postag.take(),
                        has_match,
                    ) {
                        rules.push(rule);
                    }
                    // reset
                    in_rule = false;
                    in_disambig = false;
                    action = Action::Replace;
                    has_match = false;
                }
                _ => {}
            },
            _ => {}
        }
    }

    Ok(rules)
}

#[allow(clippy::too_many_arguments)]
fn build_rule(
    id: String,
    pattern: Option<CapturedPattern>,
    antis: Vec<CapturedPattern>,
    action: Action,
    wds: Vec<Wd>,
    disambig_postag: Option<String>,
    has_match: bool,
) -> Option<DisambigRule> {
    let cap = pattern?;
    let parsed = parse_pattern(&cap.xml, cap.case_sensitive).ok()?;
    let mut unsupported = parsed.unsupported;
    if has_match {
        push_unique(&mut unsupported, "disambig-match");
    }
    // Compile antipatterns; any unsupported feature in an antipattern makes the
    // whole rule unsafe to apply (we might fire when it would have blocked).
    let mut antipatterns = Vec::new();
    for a in antis {
        match parse_pattern(&a.xml, a.case_sensitive) {
            Ok(p) => {
                for u in &p.unsupported {
                    push_unique(&mut unsupported, u);
                }
                antipatterns.push(p.pattern);
            }
            Err(_) => push_unique(&mut unsupported, "antipattern-parse"),
        }
    }
    Some(DisambigRule {
        id,
        pattern: parsed.pattern,
        antipatterns,
        action,
        wds,
        disambig_postag,
        unsupported,
    })
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.local_name().as_ref() == key.as_bytes() {
            return Some(String::from_utf8_lossy(&a.value).into_owned());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Application (the action layer)
// ---------------------------------------------------------------------------

/// Apply all rules, in order, to `sentence` (in place). Rules flagged
/// `unsupported` are skipped. Mirrors LT: each rule scans left-to-right and
/// applies at every non-overlapping match.
pub fn apply_all(rules: &[DisambigRule], sentence: &mut Vec<AnalyzedTokenReadings>) {
    for rule in rules {
        if !rule.unsupported.is_empty() {
            continue;
        }
        apply_rule(rule, sentence);
    }
}

fn apply_rule(rule: &DisambigRule, sentence: &mut [AnalyzedTokenReadings]) {
    // Precompute antipattern coverage: any token index inside an antipattern's
    // matched span blocks a rule match that overlaps it (LT antipattern
    // semantics).
    let blocked = antipattern_coverage(rule, sentence);

    let mut start = 0usize;
    while start < sentence.len() {
        let Some(m) = rule.pattern.find_at_or_after(sentence, start) else {
            break;
        };
        let overlaps_block = (m.from_token..m.to_token).any(|i| blocked.get(i).copied().unwrap_or(false));
        if !overlaps_block {
            if rule.action == Action::Filterall {
                apply_filterall(rule, sentence, &m);
            } else {
                apply_action(rule, sentence, m.marker_from_token, m.marker_to_token);
            }
        }
        // Advance past this match (non-overlapping, like LT's performer).
        start = m.to_token.max(m.from_token + 1);
    }
}

/// A boolean per sentence token: covered by some antipattern match.
fn antipattern_coverage(rule: &DisambigRule, sentence: &[AnalyzedTokenReadings]) -> Vec<bool> {
    let mut blocked = vec![false; sentence.len()];
    for anti in &rule.antipatterns {
        let mut s = 0usize;
        while s < sentence.len() {
            let Some(m) = anti.find_at_or_after(sentence, s) else {
                break;
            };
            for i in m.from_token..m.to_token {
                if i < blocked.len() {
                    blocked[i] = true;
                }
            }
            s = m.to_token.max(m.from_token + 1);
        }
    }
    blocked
}

/// `filterall`: for each marked token, keep only the readings that its
/// corresponding pattern token matched (LT `FILTERALL`). Applied only when the
/// marker maps one-to-one onto fixed (min=max=1, skip=0) pattern tokens.
fn apply_filterall(
    rule: &DisambigRule,
    sentence: &mut [AnalyzedTokenReadings],
    m: &matcher::PatternMatch,
) {
    let pat = &rule.pattern;
    let n = m.marker_to_token - m.marker_from_token;
    if pat.mark_to - pat.mark_from != n {
        return; // marker spans repetitions/skips — cannot map safely
    }
    for j in 0..n {
        let pt = &pat.tokens[pat.mark_from + j];
        if pt.min != 1 || pt.max != 1 || pt.skip != 0 || pt.is_group() {
            return;
        }
    }
    for j in 0..n {
        let pt = &pat.tokens[pat.mark_from + j];
        let tok = &mut sentence[m.marker_from_token + j];
        let ws = tok.whitespace_before;
        let kept: Vec<AnalyzedToken> = tok
            .readings
            .iter()
            .filter(|r| pt.keeps_reading(r, ws))
            .cloned()
            .collect();
        if !kept.is_empty() {
            tok.readings = kept;
        }
    }
}

fn apply_action(
    rule: &DisambigRule,
    sentence: &mut [AnalyzedTokenReadings],
    from: usize,
    to: usize,
) {
    let span = &mut sentence[from..to];
    match rule.action {
        Action::IgnoreSpelling | Action::Immunize | Action::Unify | Action::Filterall => {
            // ignore_spelling / immunize / unify do not change the printed
            // lemma/POS readings (tracked separately in LT). filterall needs the
            // pattern token's POS matcher and is deferred (left unchanged).
        }
        Action::Add => {
            // Append each <wd> as a new reading to each marked token.
            for tok in span.iter_mut() {
                for wd in &rule.wds {
                    add_reading(tok, wd);
                }
            }
        }
        Action::Remove => {
            if let Some(postag) = &rule.disambig_postag {
                // Remove readings whose POS matches the (regex) postag.
                let pm = pos_matcher(postag);
                for tok in span.iter_mut() {
                    tok.readings.retain(|r| !pos_matches(&pm, r));
                    ensure_nonempty(tok);
                }
            } else {
                for tok in span.iter_mut() {
                    for wd in &rule.wds {
                        remove_reading(tok, wd);
                    }
                    ensure_nonempty(tok);
                }
            }
        }
        Action::Filter => {
            if let Some(postag) = &rule.disambig_postag {
                // Keep only readings whose POS matches the (regex) postag.
                let pm = pos_matcher(postag);
                for tok in span.iter_mut() {
                    tok.readings.retain(|r| pos_matches(&pm, r));
                    ensure_nonempty(tok);
                }
            } else if rule.wds.len() == span.len() {
                // Keep only readings matching the per-token <wd>.
                for (tok, wd) in span.iter_mut().zip(&rule.wds) {
                    tok.readings.retain(|r| wd_matches(wd, r));
                    ensure_nonempty(tok);
                }
            }
        }
        Action::Replace => {
            if rule.wds.len() == span.len() && !rule.wds.is_empty() {
                // Per-token replacement: wd[i] becomes token[i]'s single reading.
                for (tok, wd) in span.iter_mut().zip(&rule.wds) {
                    let reading = wd_to_reading(tok, wd);
                    tok.readings = vec![reading];
                }
            } else if let Some(postag) = &rule.disambig_postag {
                // Bare <disambig postag="X">: LT's REPLACE with a disambiguatedPOS
                // touches only the FIRST marker token (`fromPos`), collapsing it
                // to a single reading `(surface, X, lemma)`, where lemma is the
                // last existing reading whose POS is exactly X (null if none).
                if let Some(tok) = span.first_mut() {
                    // Lemma: the last existing reading whose POS is exactly X;
                    // if none, the token's first reading lemma (null if untagged).
                    let lemma = tok
                        .readings
                        .iter()
                        .filter(|r| r.pos.as_deref() == Some(postag.as_str()))
                        .filter_map(|r| r.lemma.clone())
                        .last()
                        .or_else(|| tok.readings.first().and_then(|r| r.lemma.clone()));
                    tok.readings =
                        vec![AnalyzedToken::new(&tok.token, lemma.as_deref(), Some(postag))];
                }
            }
        }
    }
}

fn pos_matcher(postag: &str) -> matcher::PosMatcher {
    matcher::PosMatcher::new(postag, true)
        .unwrap_or_else(|_| matcher::PosMatcher::new("(?!x)x", true).unwrap())
}

fn pos_matches(pm: &matcher::PosMatcher, r: &AnalyzedToken) -> bool {
    // Reuse the matcher's reading test via a one-reading probe.
    pm.matches_reading(r)
}

/// Whether reading `r` matches a `<wd>` (absent attribute = wildcard).
fn wd_matches(wd: &Wd, r: &AnalyzedToken) -> bool {
    let lemma_ok = wd
        .lemma
        .as_deref()
        .map(|l| Some(l) == r.lemma.as_deref())
        .unwrap_or(true);
    let pos_ok = wd
        .pos
        .as_deref()
        .map(|p| Some(p) == r.pos.as_deref())
        .unwrap_or(true);
    lemma_ok && pos_ok
}

/// Build an `AnalyzedToken` from a `<wd>`, defaulting an empty lemma to the
/// token's surface form (LT behaviour).
fn wd_to_reading(tok: &AnalyzedTokenReadings, wd: &Wd) -> AnalyzedToken {
    let lemma = match wd.lemma.as_deref() {
        Some(l) if !l.is_empty() => l.to_string(),
        _ => tok.token.clone(),
    };
    AnalyzedToken::new(&tok.token, Some(&lemma), wd.pos.as_deref())
}

/// LT keeps a token from ever losing all readings: a `remove`/`filter` that
/// empties it leaves a single untagged reading `(∅/∅)`.
fn ensure_nonempty(tok: &mut AnalyzedTokenReadings) {
    if tok.readings.is_empty() {
        tok.readings.push(AnalyzedToken::new(&tok.token, None, None));
    }
}

/// LT `AnalyzedTokenReadings.addReading`: append the reading; if the token
/// currently has a single untagged (null-POS) reading, drop it first.
fn add_reading(tok: &mut AnalyzedTokenReadings, wd: &Wd) {
    if tok.readings.len() == 1 && tok.readings[0].has_no_tag() {
        tok.readings.clear();
    }
    tok.readings.push(wd_to_reading(tok, wd));
}

/// LT `removeReading`: drop readings matching the `<wd>` (by lemma and/or POS,
/// where an absent attribute matches anything).
fn remove_reading(tok: &mut AnalyzedTokenReadings, wd: &Wd) {
    tok.readings.retain(|r| {
        let lemma_match = wd
            .lemma
            .as_ref()
            .map(|l| Some(l.as_str()) == r.lemma.as_deref())
            .unwrap_or(true);
        let pos_match = wd
            .pos
            .as_ref()
            .map(|p| Some(p.as_str()) == r.pos.as_deref())
            .unwrap_or(true);
        !(lemma_match && pos_match)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a sentence from `(surface, &[(lemma, pos)])` tuples, prepending a
    /// SENT_START token as the raw analyzer does.
    fn sent(words: &[(&str, &[(&str, &str)])]) -> Vec<AnalyzedTokenReadings> {
        let mut v = vec![AnalyzedTokenReadings::sent_start()];
        for (i, (w, rs)) in words.iter().enumerate() {
            let readings = if rs.is_empty() {
                vec![AnalyzedToken::new(w, None, None)]
            } else {
                rs.iter()
                    .map(|(l, p)| AnalyzedToken::new(w, Some(l), Some(p)))
                    .collect()
            };
            v.push(AnalyzedTokenReadings::word(w, readings, i > 0, i));
        }
        v
    }

    /// Compile a single `<rule>` (wrapped in `<rules>`) and apply it.
    fn run(rule_xml: &str, s: &mut Vec<AnalyzedTokenReadings>) {
        let doc = format!("<rules>{rule_xml}</rules>");
        let rules = parse_disambig_rules(&doc).unwrap();
        apply_all(&rules, s);
    }

    fn readings(tok: &AnalyzedTokenReadings) -> Vec<String> {
        tok.readings
            .iter()
            .map(|r| {
                format!(
                    "{}/{}",
                    r.lemma.as_deref().unwrap_or("_"),
                    r.pos.as_deref().unwrap_or("_")
                )
            })
            .collect()
    }

    #[test]
    fn replace_wd_hard_sets_reading() {
        // ca + n't -> can/MD on "ca" (untagged); n't is glued (no space before).
        let mut s = sent(&[("ca", &[]), ("n't", &[("not", "RB")])]);
        s[2].whitespace_before = false;
        run(
            r#"<rule><pattern><marker><token>ca</token></marker>
               <token spacebefore="no">n't</token></pattern>
               <disambig action="replace"><wd lemma="can" pos="MD"/></disambig></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["can/MD"]);
    }

    #[test]
    fn bare_postag_replace_filters_to_matching_last_lemma() {
        // best[...,good/JJS,well/JJS,...] + IN -> best[well/JJS] (last matching).
        let mut s = sent(&[
            (
                "best",
                &[
                    ("best", "NN:U"),
                    ("good", "JJS"),
                    ("well", "JJS"),
                    ("well", "RBS"),
                ],
            ),
            ("of", &[("of", "IN")]),
        ]);
        run(
            r#"<rule><pattern><token>best</token><token postag="IN"/></pattern>
               <disambig postag="JJS"/></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["well/JJS"]); // only first marker token
        assert_eq!(readings(&s[2]), vec!["of/IN"]); // second token untouched
    }

    #[test]
    fn bare_postag_replace_untagged_synthesizes_null_lemma() {
        // "10" untagged -> 10[_/CD].
        let mut s = sent(&[("10", &[])]);
        run(
            r#"<rule><pattern><token regexp="yes">\d+</token></pattern>
               <disambig postag="CD"/></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["_/CD"]);
    }

    #[test]
    fn add_appends_reading() {
        // "." + add PCT -> ./. and ./PCT.
        let mut s = sent(&[(".", &[(".", ".")])]);
        run(
            r#"<rule><pattern><token regexp="yes">[.]</token></pattern>
               <disambig action="add"><wd pos="PCT"/></disambig></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["./.", "./PCT"]); // lemma defaults to surface
    }

    #[test]
    fn remove_postag_and_keeps_nonempty() {
        // "a" -> remove DT leaves a single null reading (never empty).
        let mut s = sent(&[("a", &[("a", "DT")])]);
        run(
            r#"<rule><pattern><token>a</token></pattern>
               <disambig action="remove" postag="DT"/></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["_/_"]);
    }

    #[test]
    fn filter_postag_keeps_matching() {
        // "Let" + VB -> keep VB.* readings (LET_GO).
        let mut s = sent(&[
            (
                "Let",
                &[("let", "NN"), ("let", "VB"), ("let", "VBP")],
            ),
            ("go", &[("go", "VB")]),
        ]);
        run(
            r#"<rule><pattern><marker><token>let</token></marker><token postag="VB"/></pattern>
               <disambig action="filter" postag="VB.*"/></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["let/VB", "let/VBP"]);
    }

    #[test]
    fn filterall_keeps_pattern_token_pos() {
        // six[CD,JJ,NN] marked with postag CD -> six[CD].
        let mut s = sent(&[(
            "six",
            &[("six", "CD"), ("six", "JJ"), ("six", "NN")],
        )]);
        run(
            r#"<rule><pattern><marker><token postag="CD"/></marker></pattern>
               <disambig action="filterall"/></rule>"#,
            &mut s,
        );
        assert_eq!(readings(&s[1]), vec!["six/CD"]);
    }

    #[test]
    fn antipattern_suppresses_match() {
        // Replace "have" -> VB, but an antipattern "I have" blocks it.
        let rule = r#"<rule>
            <antipattern><token regexp="yes">I|you|we|they</token><token>have</token></antipattern>
            <pattern><marker><token>have</token></marker></pattern>
            <disambig action="replace"><wd lemma="have" pos="VB"/></disambig></rule>"#;
        // Blocked context:
        let mut s = sent(&[("I", &[("I", "PRP")]), ("have", &[("have", "VBP")])]);
        run(rule, &mut s);
        assert_eq!(readings(&s[2]), vec!["have/VBP"]); // unchanged
        // Unblocked context:
        let mut s2 = sent(&[("to", &[("to", "TO")]), ("have", &[("have", "VBP")])]);
        run(rule, &mut s2);
        assert_eq!(readings(&s2[2]), vec!["have/VB"]); // replaced
    }

    #[test]
    fn scope_previous_exception_blocks_on_neighbour() {
        // Replace "one" -> CD, but an exception scope="previous">no blocks it
        // when the previous token is "no" ("no one").
        let rule = r#"<rule><pattern><marker>
            <token>one<exception scope="previous">no</exception></token>
            </marker></pattern>
            <disambig action="replace"><wd lemma="one" pos="CD"/></disambig></rule>"#;
        // Previous token is "no" -> blocked.
        let mut s = sent(&[("no", &[("no", "DT")]), ("one", &[("one", "PRP")])]);
        run(rule, &mut s);
        assert_eq!(readings(&s[2]), vec!["one/PRP"]); // unchanged
        // Different previous token -> applies.
        let mut s2 = sent(&[("just", &[("just", "RB")]), ("one", &[("one", "PRP")])]);
        run(rule, &mut s2);
        assert_eq!(readings(&s2[2]), vec!["one/CD"]);
    }

    #[test]
    fn and_group_requires_all_children() {
        // <and> requires a position with BOTH an 'install' and an 'instal' lemma.
        let rule = r#"<rule><pattern><and>
            <token inflected="yes">install</token>
            <token inflected="yes">instal</token>
            </and></pattern>
            <disambig action="remove"><wd lemma="instal"/></disambig></rule>"#;
        // Token has both lemmas -> group matches, instal removed.
        let mut s = sent(&[(
            "installs",
            &[("install", "VBZ"), ("instal", "VBZ")],
        )]);
        run(rule, &mut s);
        assert_eq!(readings(&s[1]), vec!["install/VBZ"]);
        // Token with only one lemma -> group does not match, unchanged.
        let mut s2 = sent(&[("installs", &[("install", "VBZ")])]);
        run(rule, &mut s2);
        assert_eq!(readings(&s2[1]), vec!["install/VBZ"]);
    }
}
