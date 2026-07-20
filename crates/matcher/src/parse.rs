//! Parse a LanguageTool `<pattern>` element into a [`Pattern`] of
//! [`TokenMatcher`]s. Supports the token features the English rule files use
//! most: `postag`/`postag_regexp`, `regexp`, `inflected`, `case_sensitive`,
//! `negate`/`negate_pos`, `min`/`max`/`skip`, `spacebefore`, `<exception>`, and
//! `<marker>`. `<and>`/`<or>` groups and `chunk`/`chunk_re` are not yet handled;
//! a pattern using them still parses (with those constraints dropped) and the
//! feature name is recorded in [`ParsedPattern::unsupported`] so callers can skip
//! rules they cannot yet run faithfully.

use crate::pattern::Pattern;
use crate::token::{GroupKind, PosMatcher, Scope, StringMatcher, TokenMatcher};
use quick_xml::events::{attributes::Attributes, BytesStart, Event};
use quick_xml::Reader;

pub struct ParsedPattern {
    pub pattern: Pattern,
    pub unsupported: Vec<String>,
}

pub fn parse_pattern(xml: &str, default_case_sensitive: bool) -> Result<ParsedPattern, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut tokens: Vec<TokenMatcher> = Vec::new();
    let mut unsupported: Vec<String> = Vec::new();
    let mut mark_from: Option<usize> = None;
    let mut mark_to: Option<usize> = None;

    let mut cur: Option<TokenBuild> = None;
    let mut cur_exc: Option<TokenBuild> = None;
    let mut in_pattern = false;
    // Active `<and>`/`<or>` group: its kind and the child tokens gathered so far.
    let mut group: Option<(GroupKind, Vec<TokenBuild>)> = None;

    // Finalize a completed token: into the active group, else the token list.
    macro_rules! finish_token {
        () => {
            if let Some(b) = cur.take() {
                match group.as_mut() {
                    Some((_, children)) => children.push(b),
                    None => tokens.push(b.build(&mut unsupported)),
                }
            }
        };
    }
    macro_rules! finish_exc {
        () => {
            if let (Some(b), Some(tok)) = (cur_exc.take(), cur.as_mut()) {
                tok.exceptions.push(b);
            }
        };
    }

    loop {
        match reader.read_event() {
            Err(e) => return Err(format!("XML error: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"pattern" => in_pattern = true,
                b"marker" if in_pattern => {
                    mark_from.get_or_insert(tokens.len());
                }
                b"and" if in_pattern => group = Some((GroupKind::And, Vec::new())),
                b"or" if in_pattern => group = Some((GroupKind::Or, Vec::new())),
                b"token" if in_pattern => {
                    cur = Some(TokenBuild::from_attrs(&e.attributes(), default_case_sensitive))
                }
                b"exception" => {
                    cur_exc = Some(TokenBuild::from_attrs(&e.attributes(), default_case_sensitive))
                }
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"token" if in_pattern => {
                    let b = TokenBuild::from_attrs(&e.attributes(), default_case_sensitive);
                    match group.as_mut() {
                        Some((_, children)) => children.push(b),
                        None => tokens.push(b.build(&mut unsupported)),
                    }
                }
                b"exception" => {
                    if let Some(tok) = cur.as_mut() {
                        let b = TokenBuild::from_attrs(&e.attributes(), default_case_sensitive);
                        tok.exceptions.push(b);
                    }
                }
                _ => {}
            },
            Ok(Event::Text(t)) => {
                let raw = String::from_utf8_lossy(t.as_ref()).into_owned();
                let txt = quick_xml::escape::unescape(&raw)
                    .map(|c| c.into_owned())
                    .unwrap_or(raw);
                if let Some(b) = cur_exc.as_mut() {
                    b.text.push_str(&txt);
                } else if let Some(b) = cur.as_mut() {
                    b.text.push_str(&txt);
                }
            }
            // quick-xml emits `&quot;`, `&#39;`, … inside token text as separate
            // reference events; resolve them and append to the active token.
            Ok(Event::GeneralRef(r)) => {
                if let Some(txt) = resolve_ref(&r) {
                    if let Some(b) = cur_exc.as_mut() {
                        b.text.push_str(&txt);
                    } else if let Some(b) = cur.as_mut() {
                        b.text.push_str(&txt);
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"exception" => finish_exc!(),
                b"token" => finish_token!(),
                b"and" | b"or" => {
                    if let Some((kind, children)) = group.take() {
                        tokens.push(TokenBuild::group(kind, children).build(&mut unsupported));
                    }
                }
                b"marker" => mark_to = Some(tokens.len()),
                b"pattern" => break,
                _ => {}
            },
            _ => {}
        }
    }

    let mut pattern = Pattern::new(tokens);
    if let Some(f) = mark_from {
        pattern.mark_from = f;
    }
    if let Some(t) = mark_to {
        pattern.mark_to = t;
    }
    Ok(ParsedPattern { pattern, unsupported })
}

/// Resolve an XML reference event (content between `&` and `;`): the five
/// predefined entities and numeric character references.
fn resolve_ref(r: &quick_xml::events::BytesRef) -> Option<String> {
    if let Ok(Some(ch)) = r.resolve_char_ref() {
        return Some(ch.to_string());
    }
    match String::from_utf8_lossy(r.as_ref()).as_ref() {
        "quot" => Some("\"".to_string()),
        "amp" => Some("&".to_string()),
        "apos" => Some("'".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        _ => None,
    }
}

fn note(list: &mut Vec<String>, name: &str) {
    if !list.iter().any(|s| s == name) {
        list.push(name.to_string());
    }
}

struct TokenBuild {
    text: String,
    regexp: bool,
    inflected: bool,
    negate: bool,
    negate_pos: bool,
    case_sensitive: bool,
    postag: Option<String>,
    postag_regexp: bool,
    spacebefore: Option<bool>,
    min: i32,
    max: i32,
    skip: i32,
    has_chunk: bool,
    /// `scope="previous"|"next"|"current"` on an `<exception>`.
    scope: Scope,
    exceptions: Vec<TokenBuild>,
    /// When set, this builder is an `<and>`/`<or>` group of child tokens.
    group: Option<(GroupKind, Vec<TokenBuild>)>,
}

impl TokenBuild {
    /// An `<and>`/`<or>` group builder (no attributes; a single position).
    fn group(kind: GroupKind, children: Vec<TokenBuild>) -> Self {
        let mut b = TokenBuild::empty();
        b.group = Some((kind, children));
        b
    }

    fn empty() -> Self {
        TokenBuild {
            text: String::new(),
            regexp: false,
            inflected: false,
            negate: false,
            negate_pos: false,
            case_sensitive: false,
            postag: None,
            postag_regexp: false,
            spacebefore: None,
            min: 1,
            max: 1,
            skip: 0,
            has_chunk: false,
            scope: Scope::Current,
            exceptions: Vec::new(),
            group: None,
        }
    }

    fn from_attrs(attrs: &Attributes, default_cs: bool) -> Self {
        let get = |k: &str| attr(attrs, k);
        let max = match get("max").as_deref() {
            Some("unlimited") => -1,
            Some(v) => v.parse().unwrap_or(1),
            None => 1,
        };
        TokenBuild {
            text: String::new(),
            regexp: get("regexp").as_deref() == Some("yes"),
            inflected: get("inflected").as_deref() == Some("yes"),
            negate: get("negate").as_deref() == Some("yes"),
            negate_pos: get("negate_pos").as_deref() == Some("yes"),
            case_sensitive: get("case_sensitive").map(|v| v == "yes").unwrap_or(default_cs),
            postag: get("postag"),
            postag_regexp: get("postag_regexp").as_deref() == Some("yes"),
            spacebefore: get("spacebefore").map(|v| v == "yes"),
            min: get("min").and_then(|v| v.parse().ok()).unwrap_or(1),
            max,
            skip: get("skip").and_then(|v| v.parse().ok()).unwrap_or(0),
            has_chunk: get("chunk").is_some() || get("chunk_re").is_some(),
            scope: match get("scope").as_deref() {
                Some("previous") => Scope::Previous,
                Some("next") => Scope::Next,
                _ => Scope::Current,
            },
            exceptions: Vec::new(),
            group: None,
        }
    }

    fn build(self, unsupported: &mut Vec<String>) -> TokenMatcher {
        // `<and>`/`<or>` group: build children; the group occupies one position.
        if let Some((kind, children)) = self.group {
            return TokenMatcher {
                group_kind: Some(kind),
                group: children.into_iter().map(|c| c.build(unsupported)).collect(),
                min: self.min,
                max: self.max,
                skip: self.skip,
                ..Default::default()
            };
        }
        if self.has_chunk {
            note(unsupported, "chunk");
        }
        let text = self.text.trim();
        let string = if text.is_empty() {
            None
        } else if self.regexp {
            match StringMatcher::regex(text, self.case_sensitive) {
                Ok(m) => Some(m),
                Err(_) => {
                    // A regex we cannot compile must not silently become a
                    // match-anything token; flag the rule so callers skip it.
                    note(unsupported, "regex");
                    None
                }
            }
        } else {
            Some(StringMatcher::literal(text, self.case_sensitive))
        };
        let pos = self
            .postag
            .as_deref()
            .and_then(|p| PosMatcher::new(p, self.postag_regexp).ok());
        TokenMatcher {
            string,
            inflected: self.inflected,
            negate: self.negate,
            pos,
            negate_pos: self.negate_pos,
            spacebefore: self.spacebefore,
            scope: self.scope,
            exceptions: self
                .exceptions
                .into_iter()
                .map(|e| e.build(unsupported))
                .collect(),
            min: self.min,
            max: self.max,
            skip: self.skip,
            group_kind: None,
            group: Vec::new(),
        }
    }
}

fn attr(attrs: &Attributes, key: &str) -> Option<String> {
    for a in attrs.clone().flatten() {
        if a.key.local_name().as_ref() == key.as_bytes() {
            return Some(String::from_utf8_lossy(&a.value).into_owned());
        }
    }
    None
}

// Silence an unused-import lint if BytesStart isn't referenced elsewhere.
#[allow(dead_code)]
fn _uses(_: &BytesStart) {}
