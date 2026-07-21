//! Phase 1.4 grammar engine: `en/grammar.xml` `<rule>`s on the shared matcher.
//!
//! A grammar rule is a `<pattern>` (+ `<antipattern>`s) plus a `<message>` that
//! contains `<suggestion>`s; matching a sentence yields the `matches[]` the
//! `/v2/check` contract exposes (offset/length, message, replacements, rule id,
//! category id). `\N` in a message/suggestion is a backreference to the surface
//! of the N-th pattern token. `<match>` transforms (POS-driven replacements) are
//! not yet supported — a rule using one is flagged and skipped.

use fancy_regex::Regex;
use matcher::{parse_pattern, AnalyzedTokenReadings, Pattern};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

/// One rendered grammar finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarMatch {
    /// Character offset of the marked span in the full text.
    pub offset: usize,
    /// Character length of the marked span.
    pub length: usize,
    pub message: String,
    pub replacements: Vec<String>,
    pub rule_id: String,
    pub category_id: String,
}

/// A `\N` backreference, literal text fragment, or a `<match>` element.
#[derive(Clone)]
enum Seg {
    Text(String),
    Ref(usize), // 1-based pattern-token index
    Match(MatchSpec),
}

/// `case_conversion` on a `<match>` element.
#[derive(Clone, Copy)]
enum CaseConv {
    None,
    StartLower,
    StartUpper,
    AllLower,
    AllUpper,
    /// Apply the reference token's own case pattern to the produced form.
    Preserve,
}

/// A `<match no="N" …>` in a `<suggestion>`/`<message>`, minus POS synthesis:
/// takes the surface of pattern token `no`, optionally applies a `regexp_match`
/// → `regexp_replace` substitution, then a `case_conversion`. A `<match>` that
/// carries `postag`/`postag_replace` needs the Morfologik synthesizer we don't
/// have yet, so its rule is flagged `match-synth` and skipped instead.
#[derive(Clone)]
struct MatchSpec {
    no: usize,
    case_conv: CaseConv,
    regex: Option<(Regex, String)>,
}

/// One piece of a `<message>`: literal/ref text, or an inline `<suggestion>`.
#[derive(Clone)]
enum MsgItem {
    Seg(Seg),
    Suggestion(Vec<Seg>),
}

/// A `<example>`: input text, the marked span (char offset/length in `text`),
/// and the expected `correction` alternatives (`None` = negative example, the
/// rule must not fire).
#[derive(Debug, Clone)]
pub struct Example {
    pub text: String,
    pub marker: Option<(usize, usize)>,
    pub correction: Option<Vec<String>>,
    /// `type="triggers_error"`: LT documents that the rule *does* fire here (a
    /// known/accepted match), so it is neither a positive nor a plain negative.
    pub triggers_error: bool,
}

pub struct GrammarRule {
    pub id: String,
    pub category_id: String,
    pub pattern: Pattern,
    pub antipatterns: Vec<Pattern>,
    message: Vec<MsgItem>,
    /// All `<suggestion>` blocks in document order (in-message and standalone) —
    /// these are the `replacements[]`.
    suggestions: Vec<Vec<Seg>>,
    pub examples: Vec<Example>,
    pub unsupported: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

struct Captured {
    xml: String,
    case_sensitive: bool,
}

/// Parse a whole `grammar.xml` (already entity-expanded) into rules.
pub fn parse_grammar_rules(xml: &str) -> Result<Vec<GrammarRule>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut rules: Vec<GrammarRule> = Vec::new();

    let mut category_id = String::new();
    let mut group_id: Option<String> = None;
    let mut in_rule = false;
    let mut rule_id_attr: Option<String> = None;
    // Disabled at `level=default`: `default="off"` or `tags="picky"` on the
    // category/rulegroup/rule (a rule-level `default="on"` re-enables).
    let mut category_off = false;
    let mut group_off = false;
    let mut rule_off = false;

    let mut captured_pattern: Option<Captured> = None;
    let mut captured_antis: Vec<Captured> = Vec::new();
    // Antipatterns declared at rulegroup level apply to every rule in the group.
    let mut group_antis: Vec<Captured> = Vec::new();

    let mut capture: Option<Writer<Vec<u8>>> = None;
    let mut capture_is_anti = false;
    let mut capture_cs = false;

    // Message / suggestion accumulation.
    let mut in_message = false;
    let mut message: Vec<MsgItem> = Vec::new();
    let mut in_suggestion = false;
    let mut sugg: Vec<Seg> = Vec::new();
    let mut suggestions: Vec<Vec<Seg>> = Vec::new();
    let mut sugg_in_message = false;
    // A `<match postag=…>` needs the (not-yet-built) synthesizer.
    let mut needs_synth = false;
    // Inside a `<match>…</match>` (start form) — swallow its lemma-override text.
    let mut in_match = false;
    // A `<filter class=...>` needs a Java filter class we cannot run.
    let mut has_filter = false;

    // Example accumulation.
    let mut examples: Vec<Example> = Vec::new();
    let mut in_example = false;
    let mut ex_text = String::new();
    let mut ex_correction: Option<Vec<String>> = None;
    let mut ex_marker: Option<(usize, usize)> = None;
    let mut ex_marker_start: Option<usize> = None;
    let mut ex_triggers_error = false;

    let is_wrapper = |n: &[u8]| n == b"pattern" || n == b"antipattern";

    macro_rules! push_text {
        ($txt:expr) => {{
            let segs = parse_backrefs(&$txt);
            if in_match {
                // lemma-override text of a synth `<match>` — never rendered.
            } else if in_suggestion {
                sugg.extend(segs);
            } else if in_message {
                message.extend(segs.into_iter().map(MsgItem::Seg));
            }
        }};
    }
    // Text/refs land in a suggestion, the message, or an example.
    macro_rules! in_text_ctx {
        () => {
            in_message || in_suggestion || in_example
        };
    }

    loop {
        let ev = reader.read_event().map_err(|e| format!("XML error: {e}"))?;

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
                b"category" => {
                    category_id = attr(e, "id").unwrap_or_default();
                    category_off = is_disabled(e);
                }
                b"rulegroup" => {
                    group_id = attr(e, "id");
                    group_antis.clear();
                    group_off = is_disabled(e);
                }
                b"rule" => {
                    in_rule = true;
                    rule_id_attr = attr(e, "id");
                    // A rule-level default="on" re-enables an off group/category.
                    rule_off = match attr(e, "default").as_deref() {
                        Some("on") => false,
                        Some("off") => true,
                        _ => group_off || category_off,
                    } || attr(e, "tags").map(|t| t.contains("picky")).unwrap_or(false);
                    captured_pattern = None;
                    captured_antis.clear();
                    message.clear();
                    suggestions.clear();
                    needs_synth = false;
                    has_filter = false;
                }
                b"pattern" | b"antipattern" if in_rule || group_id.is_some() => {
                    capture_is_anti = e.local_name().as_ref() == b"antipattern";
                    capture_cs = attr(e, "case_sensitive").as_deref() == Some("yes");
                    let mut w = Writer::new(Vec::new());
                    let _ = w.write_event(Event::Start(quick_xml::events::BytesStart::new(
                        "pattern",
                    )));
                    capture = Some(w);
                }
                b"message" if in_rule => {
                    in_message = true;
                    message.clear();
                }
                b"suggestion" if in_rule => {
                    in_suggestion = true;
                    sugg = Vec::new();
                    sugg_in_message = in_message;
                }
                b"match" if in_suggestion || in_message => {
                    match parse_match_spec(e) {
                        Ok(spec) => push_match(spec, in_suggestion, &mut sugg, &mut message),
                        Err(()) => needs_synth = true,
                    }
                    in_match = true; // start form: skip any lemma-override text
                }
                b"filter" if in_rule => has_filter = true,
                b"example" if in_rule => {
                    in_example = true;
                    ex_text.clear();
                    ex_marker = None;
                    ex_marker_start = None;
                    ex_triggers_error = attr(e, "type").as_deref() == Some("triggers_error");
                    ex_correction = attr(e, "correction").map(|c| {
                        // Unescape XML entities (e.g. `&apos;`) before splitting.
                        let c = quick_xml::escape::unescape(&c)
                            .map(|x| x.into_owned())
                            .unwrap_or(c);
                        c.split('|').map(|s| s.to_string()).collect::<Vec<_>>()
                    });
                }
                b"marker" if in_example => {
                    ex_marker_start = Some(ex_text.chars().count());
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                b"match" if in_suggestion || in_message => match parse_match_spec(e) {
                    Ok(spec) => push_match(spec, in_suggestion, &mut sugg, &mut message),
                    Err(()) => needs_synth = true,
                },
                b"filter" if in_rule => has_filter = true,
                _ => {}
            },
            Event::Text(t) => {
                if in_text_ctx!() {
                    let raw = String::from_utf8_lossy(t.as_ref()).into_owned();
                    let txt = quick_xml::escape::unescape(&raw)
                        .map(|c| c.into_owned())
                        .unwrap_or(raw);
                    if in_example {
                        ex_text.push_str(&txt);
                    } else {
                        push_text!(txt);
                    }
                }
            }
            Event::GeneralRef(r) => {
                if in_text_ctx!() {
                    if let Some(txt) = resolve_ref(r) {
                        if in_example {
                            ex_text.push_str(&txt);
                        } else {
                            push_text!(txt);
                        }
                    }
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"pattern" | b"antipattern" if capture.is_some() => {
                    let mut w = capture.take().unwrap();
                    let _ = w.write_event(Event::End(quick_xml::events::BytesEnd::new("pattern")));
                    let xml = String::from_utf8_lossy(&w.into_inner()).into_owned();
                    let cap = Captured {
                        xml,
                        case_sensitive: capture_cs,
                    };
                    if capture_is_anti {
                        if in_rule {
                            captured_antis.push(cap);
                        } else {
                            group_antis.push(cap); // rulegroup-level antipattern
                        }
                    } else {
                        captured_pattern = Some(cap);
                    }
                }
                b"match" => in_match = false,
                b"suggestion" if in_suggestion => {
                    in_suggestion = false;
                    let block = std::mem::take(&mut sugg);
                    if sugg_in_message {
                        message.push(MsgItem::Suggestion(block.clone()));
                    }
                    suggestions.push(block);
                }
                b"message" => in_message = false,
                b"marker" if in_example => {
                    if let Some(s) = ex_marker_start.take() {
                        ex_marker = Some((s, ex_text.chars().count() - s));
                    }
                }
                b"example" if in_example => {
                    in_example = false;
                    examples.push(Example {
                        text: std::mem::take(&mut ex_text),
                        marker: ex_marker.take(),
                        correction: ex_correction.take(),
                        triggers_error: std::mem::take(&mut ex_triggers_error),
                    });
                }
                b"rule" if in_rule => {
                    let id = rule_id_attr
                        .clone()
                        .or_else(|| group_id.clone())
                        .unwrap_or_default();
                    if let Some(mut rule) = build_rule(
                        id,
                        category_id.clone(),
                        captured_pattern.take(),
                        &captured_antis,
                        &group_antis,
                        std::mem::take(&mut message),
                        std::mem::take(&mut suggestions),
                        std::mem::take(&mut examples),
                        needs_synth || has_filter,
                    ) {
                        if rule_off {
                            push_unique(&mut rule.unsupported, "disabled");
                        }
                        rules.push(rule);
                    }
                    in_rule = false;
                    rule_id_attr = None;
                }
                b"rulegroup" => {
                    group_id = None;
                    group_off = false;
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
    category_id: String,
    pattern: Option<Captured>,
    antis: &[Captured],
    group_antis: &[Captured],
    message: Vec<MsgItem>,
    suggestions: Vec<Vec<Seg>>,
    examples: Vec<Example>,
    needs_synth: bool,
) -> Option<GrammarRule> {
    let cap = pattern?;
    let parsed = parse_pattern(&cap.xml, cap.case_sensitive).ok()?;
    let mut unsupported = parsed.unsupported;
    if needs_synth {
        push_unique(&mut unsupported, "match-synth");
    }
    let mut antipatterns = Vec::new();
    for a in antis.iter().chain(group_antis.iter()) {
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
    Some(GrammarRule {
        id,
        category_id,
        pattern: parsed.pattern,
        antipatterns,
        message,
        suggestions,
        examples,
        unsupported,
    })
}

/// Split text into literal/`\N` segments.
fn parse_backrefs(s: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(d) = chars.peek().copied().filter(|d| d.is_ascii_digit()) {
                let mut num = String::new();
                num.push(d);
                chars.next();
                while let Some(d2) = chars.peek().copied().filter(|d| d.is_ascii_digit()) {
                    num.push(d2);
                    chars.next();
                }
                if !cur.is_empty() {
                    out.push(Seg::Text(std::mem::take(&mut cur)));
                }
                out.push(Seg::Ref(num.parse().unwrap_or(0)));
                continue;
            }
        }
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(Seg::Text(cur));
    }
    out
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

/// A category/rulegroup element disabled at `level=default`.
fn is_disabled(e: &quick_xml::events::BytesStart) -> bool {
    attr(e, "default").as_deref() == Some("off")
        || attr(e, "tags").map(|t| t.contains("picky")).unwrap_or(false)
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    for a in e.attributes().flatten() {
        if a.key.local_name().as_ref() == key.as_bytes() {
            return Some(String::from_utf8_lossy(&a.value).into_owned());
        }
    }
    None
}

/// Parse a `<match>` element into a [`MatchSpec`], or `Err(())` if it needs the
/// synthesizer (`postag`/`postag_replace`) or references nothing (`no` missing).
fn parse_match_spec(e: &BytesStart) -> Result<MatchSpec, ()> {
    if attr(e, "postag").is_some() || attr(e, "postag_replace").is_some() {
        return Err(());
    }
    let no: usize = attr(e, "no").and_then(|s| s.parse().ok()).unwrap_or(0);
    if no == 0 {
        return Err(()); // `no="0"` is a whole-token ref we can't resolve here
    }
    let case_conv = match attr(e, "case_conversion").as_deref() {
        Some("startlower") => CaseConv::StartLower,
        Some("startupper") => CaseConv::StartUpper,
        Some("alllower") => CaseConv::AllLower,
        Some("allupper") => CaseConv::AllUpper,
        Some("preserve") => CaseConv::Preserve,
        _ => CaseConv::None,
    };
    let regex = match (attr(e, "regexp_match"), attr(e, "regexp_replace")) {
        (Some(m), Some(r)) => {
            let re = Regex::new(&m).map_err(|_| ())?;
            Some((re, java_to_fancy_replacement(&r)))
        }
        _ => None,
    };
    Ok(MatchSpec { no, case_conv, regex })
}

/// Translate a Java `Matcher.replaceAll` template to fancy-regex's expander
/// syntax. Java reads `$` + maximal digits as a group ref (`$1re` = group 1 then
/// literal `re`), while fancy-regex would greedily parse `1re` as a group *name*;
/// wrapping numbered groups as `${N}` disambiguates. `\x` is a Java escape → the
/// literal `x`; a lone `$` becomes `$$` (fancy's literal-dollar).
fn java_to_fancy_replacement(r: &str) -> String {
    let mut out = String::new();
    let mut chars = r.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '$' => {
                let mut num = String::new();
                while let Some(d) = chars.peek().copied().filter(|d| d.is_ascii_digit()) {
                    num.push(d);
                    chars.next();
                }
                if num.is_empty() {
                    out.push_str("$$"); // literal dollar
                } else {
                    out.push_str("${");
                    out.push_str(&num);
                    out.push('}');
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Route a parsed match into the active suggestion or message stream.
fn push_match(spec: MatchSpec, in_suggestion: bool, sugg: &mut Vec<Seg>, message: &mut Vec<MsgItem>) {
    if in_suggestion {
        sugg.push(Seg::Match(spec));
    } else {
        message.push(MsgItem::Seg(Seg::Match(spec)));
    }
}

fn resolve_ref(r: &quick_xml::events::BytesRef) -> Option<String> {
    if let Ok(Some(ch)) = r.resolve_char_ref() {
        return Some(ch.to_string());
    }
    match String::from_utf8_lossy(r.as_ref()).as_ref() {
        "quot" => Some("\"".into()),
        "amp" => Some("&".into()),
        "apos" => Some("'".into()),
        "lt" => Some("<".into()),
        "gt" => Some(">".into()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Matching / rendering
// ---------------------------------------------------------------------------

/// Run all supported rules over one analyzed sentence, returning matches with
/// character offsets relative to the full text (`sentence_char_offset` is the
/// char offset of this sentence's start within the whole text).
pub fn check_sentence(
    rules: &[GrammarRule],
    tokens: &[AnalyzedTokenReadings],
    sentence_text: &str,
    sentence_char_offset: usize,
) -> Vec<GrammarMatch> {
    let mut out = Vec::new();
    for rule in rules {
        if !rule.unsupported.is_empty() {
            continue;
        }
        collect_rule_matches(rule, tokens, sentence_text, sentence_char_offset, &mut out);
    }
    filter_overlaps(out)
}

/// LT drops a match whose span is entirely contained within another match's
/// span (keeping the wider one). Applied across all rules for a sentence.
fn filter_overlaps(mut matches: Vec<GrammarMatch>) -> Vec<GrammarMatch> {
    // Widest first so a container is seen before the matches it covers.
    matches.sort_by(|a, b| {
        a.offset
            .cmp(&b.offset)
            .then(b.length.cmp(&a.length))
    });
    let mut kept: Vec<GrammarMatch> = Vec::new();
    for m in matches {
        let m_end = m.offset + m.length;
        let contained = kept.iter().any(|k| {
            let k_end = k.offset + k.length;
            k.offset <= m.offset && m_end <= k_end && (k.offset, k_end) != (m.offset, m_end)
        });
        if !contained {
            kept.push(m);
        }
    }
    kept
}

/// Run a single rule over one analyzed sentence (used by the example oracle).
pub fn check_one(
    rule: &GrammarRule,
    tokens: &[AnalyzedTokenReadings],
    sentence_text: &str,
    base: usize,
) -> Vec<GrammarMatch> {
    let mut out = Vec::new();
    if rule.unsupported.is_empty() {
        collect_rule_matches(rule, tokens, sentence_text, base, &mut out);
    }
    out
}

fn collect_rule_matches(
    rule: &GrammarRule,
    tokens: &[AnalyzedTokenReadings],
    sentence_text: &str,
    base: usize,
    out: &mut Vec<GrammarMatch>,
) {
    let blocked = antipattern_coverage(&rule.antipatterns, tokens);
    let mut start = 0usize;
    while start < tokens.len() {
        let Some(m) = rule.pattern.find_at_or_after(tokens, start) else {
            break;
        };
        let overlaps =
            (m.from_token..m.to_token).any(|i| blocked.get(i).copied().unwrap_or(false));
        if !overlaps {
            if let Some(gm) = render_match(rule, tokens, &m, sentence_text, base) {
                out.push(gm);
            }
        }
        start = m.to_token.max(m.from_token + 1);
    }
}

fn antipattern_coverage(antis: &[Pattern], tokens: &[AnalyzedTokenReadings]) -> Vec<bool> {
    let mut blocked = vec![false; tokens.len()];
    for anti in antis {
        let mut s = 0usize;
        while s < tokens.len() {
            let Some(m) = anti.find_at_or_after(tokens, s) else {
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

fn render_match(
    rule: &GrammarRule,
    tokens: &[AnalyzedTokenReadings],
    m: &matcher::PatternMatch,
    sentence_text: &str,
    base: usize,
) -> Option<GrammarMatch> {
    // Marker byte span -> char offset/length within the sentence.
    let from_tok = &tokens[m.marker_from_token];
    let last_tok = &tokens[m.marker_to_token.saturating_sub(1).max(m.marker_from_token)];
    let start_byte = from_tok.start_pos;
    let end_byte = last_tok.start_pos + last_tok.token.len();
    if end_byte > sentence_text.len() {
        return None;
    }
    let offset = base + sentence_text[..start_byte].chars().count();
    let length = sentence_text[start_byte..end_byte].chars().count();

    let resolve = |n: usize| -> String {
        // \N / `<match no=N>` is 1-based over pattern tokens. LT clamps an
        // out-of-range ref to the last matched token (MatchState.setToken).
        let last = m.element_start.len().saturating_sub(1);
        let idx = n.saturating_sub(1).min(last);
        let (Some(&start), Some(&count)) =
            (m.element_start.get(idx), m.element_count.get(idx))
        else {
            return String::new();
        };
        // A `min="0"` element that matched nothing has an empty reference; an
        // element that matched several tokens joins them with their whitespace.
        let mut s = String::new();
        for i in start..start + count {
            let Some(t) = tokens.get(i) else { break };
            if i > start && t.whitespace_before {
                s.push(' ');
            }
            s.push_str(&t.token);
        }
        s
    };

    // LT preserves case: when the marked text starts uppercase, a suggestion
    // that starts lowercase is capitalized to match.
    let upper_initial = from_tok
        .token
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false);
    let replacements: Vec<String> = rule
        .suggestions
        .iter()
        .map(|segs| {
            let s = normalize_spaces(&expand(segs, &resolve));
            // LT capitalizes a suggestion when the error is uppercase-initial,
            // *unless* the suggestion opens with a `\N` backref and one of its
            // `<match>`es converts case (PatternRuleMatcher.matchPreservesCase).
            if upper_initial && !suppresses_capitalize(segs) {
                capitalize_first(&s)
            } else {
                s
            }
        })
        .collect();
    let message = render_message(&rule.message, &resolve);

    Some(GrammarMatch {
        offset,
        length,
        message,
        replacements,
        rule_id: rule.id.clone(),
        category_id: rule.category_id.clone(),
    })
}

/// Capitalize the first character if it is a lowercase letter (LT case-preserve).
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_lowercase() => c.to_uppercase().chain(chars).collect(),
        _ => s.to_string(),
    }
}

fn expand(segs: &[Seg], resolve: &dyn Fn(usize) -> String) -> String {
    let mut s = String::new();
    for seg in segs {
        match seg {
            Seg::Text(t) => s.push_str(t),
            Seg::Ref(n) => s.push_str(&resolve(*n)),
            Seg::Match(spec) => s.push_str(&apply_match_spec(spec, resolve)),
        }
    }
    s
}

/// LT suppresses its outer "capitalize when the error is uppercase-initial" for
/// a suggestion that opens with a `\N` backreference *and* contains a `<match>`
/// with an explicit `case_conversion` (`matchPreservesCase` returns false).
fn suppresses_capitalize(segs: &[Seg]) -> bool {
    let starts_with_ref = matches!(segs.first(), Some(Seg::Ref(_)));
    let has_caseconv_match = segs
        .iter()
        .any(|s| matches!(s, Seg::Match(m) if !matches!(m.case_conv, CaseConv::None)));
    starts_with_ref && has_caseconv_match
}

/// Collapse runs of whitespace to a single space, as LT does when an empty
/// backreference leaves a double space (`concatWithoutExtraSpace`). Leading and
/// trailing single spaces are preserved (some suggestions insert one on purpose).
fn normalize_spaces(s: &str) -> String {
    let mut out = String::new();
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

/// Render one `<match>`: surface of token `no`, `regexp` substitution, then case.
fn apply_match_spec(spec: &MatchSpec, resolve: &dyn Fn(usize) -> String) -> String {
    let source = resolve(spec.no);
    let mut s = source.clone();
    if let Some((re, rep)) = &spec.regex {
        s = re.replace_all(&s, rep.as_str()).into_owned();
    }
    match spec.case_conv {
        CaseConv::None => s,
        CaseConv::AllLower => s.to_lowercase(),
        CaseConv::AllUpper => s.to_uppercase(),
        CaseConv::StartLower => lower_first(&s),
        CaseConv::StartUpper => upper_first(&s),
        CaseConv::Preserve => preserve_case(&source, &s),
    }
}

/// Force-lowercase the first character.
fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Force-uppercase the first character.
fn upper_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Apply `original`'s case shape to `s` (LT `case_conversion="preserve"`): all
/// upper if `original` is all-upper, else start-upper if it is capitalized.
fn preserve_case(original: &str, s: &str) -> String {
    let letters: Vec<char> = original.chars().filter(|c| c.is_alphabetic()).collect();
    if !letters.is_empty() && letters.iter().all(|c| c.is_uppercase()) {
        s.to_uppercase()
    } else if original.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        upper_first(s)
    } else {
        s.to_string()
    }
}

fn render_message(items: &[MsgItem], resolve: &dyn Fn(usize) -> String) -> String {
    let mut s = String::new();
    for it in items {
        match it {
            MsgItem::Seg(Seg::Text(t)) => s.push_str(t),
            MsgItem::Seg(Seg::Ref(n)) => s.push_str(&resolve(*n)),
            MsgItem::Seg(Seg::Match(spec)) => s.push_str(&apply_match_spec(spec, resolve)),
            MsgItem::Suggestion(segs) => {
                s.push('\u{201C}');
                s.push_str(&expand(segs, resolve));
                s.push('\u{201D}');
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use matcher::{AnalyzedToken, AnalyzedTokenReadings};

    fn sent_from(text: &str, words: &[(&str, &str, &str)]) -> Vec<AnalyzedTokenReadings> {
        // Build tokens with real byte offsets by finding each surface in `text`.
        let mut v = vec![AnalyzedTokenReadings::sent_start()];
        let mut cursor = 0usize;
        for (i, (w, l, p)) in words.iter().enumerate() {
            let byte = text[cursor..].find(w).map(|o| cursor + o).unwrap_or(cursor);
            cursor = byte + w.len();
            v.push(AnalyzedTokenReadings::word(
                w,
                vec![AnalyzedToken::new(w, Some(l), Some(p))],
                i > 0,
                byte,
            ));
        }
        v
    }

    #[test]
    fn modal_of_backref_suggestions() {
        let doc = r#"<rules><category id="TYPOS"><rulegroup id="MODAL_OF">
          <rule><pattern>
            <marker><token regexp="yes">would|could</token><token>of</token></marker>
            <token postag="VBN"/>
          </pattern>
          <message>Use <suggestion>\1 have</suggestion> or <suggestion>\1've</suggestion>.</message>
          </rule></rulegroup></category></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        assert_eq!(rules.len(), 1);
        let text = "I would of gone.";
        let toks = sent_from(text, &[
            ("I", "I", "PRP"),
            ("would", "would", "MD"),
            ("of", "of", "IN"),
            ("gone", "go", "VBN"),
        ]);
        let ms = check_sentence(&rules, &toks, text, 0);
        assert_eq!(ms.len(), 1);
        let m = &ms[0];
        assert_eq!(m.rule_id, "MODAL_OF");
        assert_eq!(m.category_id, "TYPOS");
        assert_eq!((m.offset, m.length), (2, 8)); // "would of"
        assert_eq!(m.replacements, vec!["would have".to_string(), "would've".to_string()]);
    }

    #[test]
    fn postag_match_needs_synth() {
        // A `<match postag=…>` requires the synthesizer -> flagged, skipped.
        let doc = r#"<rules><rule><pattern><token>x</token></pattern>
          <message>See <suggestion><match no="1" postag="VBZ"/></suggestion>.</message></rule></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        assert!(rules[0].unsupported.contains(&"match-synth".to_string()));
    }

    #[test]
    fn plain_match_is_supported_and_copies_token() {
        // `<match no="1"/>` with no transform == `\1`.
        let doc = r#"<rules><category id="X"><rule id="R"><pattern>
            <marker><token regexp="yes">teh</token></marker></pattern>
          <message>Did you mean <suggestion><match no="1"/></suggestion>?</message>
          </rule></category></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        assert!(rules[0].unsupported.is_empty());
        let text = "teh cat";
        let toks = sent_from(text, &[("teh", "teh", "NN"), ("cat", "cat", "NN")]);
        let ms = check_sentence(&rules, &toks, text, 0);
        assert_eq!(ms[0].replacements, vec!["teh".to_string()]);
    }

    #[test]
    fn empty_optional_backref_and_space_collapse() {
        // `\2` for a min="0" token that matched nothing is empty; the resulting
        // double space is collapsed (FOR_AWHILE: "in while" -> "in a while").
        let doc = r#"<rules><category id="X"><rule id="R"><pattern>
            <marker><token>in</token><token min="0">quite</token><token>while</token></marker>
            <token>.</token></pattern>
          <message>Did you mean <suggestion>\1 \2 a \3</suggestion>?</message>
          </rule></category></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        let text = "in while .";
        let toks = sent_from(text, &[("in", "in", "IN"), ("while", "while", "NN"), (".", ".", "PCT")]);
        let ms = check_sentence(&rules, &toks, text, 0);
        assert_eq!(ms[0].replacements, vec!["in a while".to_string()]);
    }

    #[test]
    fn out_of_range_no_clamps_to_last() {
        // `no="2"` on a single-token pattern clamps to the last token (THE_DUTCH).
        let doc = r#"<rules><category id="X"><rule id="R"><pattern>
            <marker><token case_sensitive="yes">dutch</token></marker></pattern>
          <message><suggestion><match no="2" case_conversion="startupper"/></suggestion></message>
          </rule></category></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        let text = "dutch people";
        let toks = sent_from(text, &[("dutch", "dutch", "JJ"), ("people", "people", "NNS")]);
        let ms = check_sentence(&rules, &toks, text, 0);
        assert_eq!(ms[0].replacements, vec!["Dutch".to_string()]);
    }

    #[test]
    fn java_replacement_group_adjacent_text() {
        // `$1re` is Java "group 1 then literal re", not fancy group name "1re".
        assert_eq!(java_to_fancy_replacement("$1re"), "${1}re");
        assert_eq!(java_to_fancy_replacement("$1 $3"), "${1} ${3}");
        assert_eq!(java_to_fancy_replacement(r"\$"), "$");
    }

    #[test]
    fn match_case_conversion_and_regex() {
        // startupper case conversion, and a regexp substitution, on token 1.
        let doc = r#"<rules><category id="X"><rulegroup id="R">
          <rule><pattern>
            <marker><token regexp="yes">iphone</token></marker></pattern>
          <message><suggestion><match no="1" case_conversion="startupper"/></suggestion></message>
          </rule>
          <rule><pattern>
            <marker><token regexp="yes">colour</token></marker></pattern>
          <message><suggestion><match no="1" regexp_match="(?i)our$" regexp_replace="or"/></suggestion></message>
          </rule></rulegroup></category></rules>"#;
        let rules = parse_grammar_rules(doc).unwrap();
        let text = "iphone colour";
        let toks = sent_from(text, &[("iphone", "iphone", "NN"), ("colour", "colour", "NN")]);
        let ms = check_sentence(&rules, &toks, text, 0);
        // startupper -> "Iphone"; regex our->or -> "color"
        let repls: Vec<_> = ms.iter().flat_map(|m| m.replacements.clone()).collect();
        assert!(repls.contains(&"Iphone".to_string()), "{repls:?}");
        assert!(repls.contains(&"color".to_string()), "{repls:?}");
    }
}
