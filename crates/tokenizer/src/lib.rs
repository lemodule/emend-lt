//! English word tokenizer, a port of LanguageTool's `EnglishWordTokenizer`
//! (which extends `WordTokenizer`).
//!
//! NOTE ON A REAL DEPENDENCY: LT's tokenizer calls the English POS tagger inside
//! `wordsToAdd` — a token containing `-`/`'` is kept whole iff the tagger already
//! recognises it (e.g. `n't`, `o'clock`), otherwise it is split on apostrophes.
//! That coupling is Phase 1.3. Here the tagger is injected as the [`WordTagger`]
//! trait so the tokenizer is complete now and becomes byte-exact once the real
//! tagger (built on the Morfologik reader) is wired in.

use fancy_regex::Regex;
use std::sync::OnceLock;

/// The tagger dependency: does the English dictionary recognise `word` (any
/// analysis)? Implemented in Phase 1.3 by the Morfologik-backed POS tagger.
pub trait WordTagger {
    fn is_tagged(&self, word: &str) -> bool;
}

/// A tagger that recognises nothing — for tests / tagger-independent behaviour.
pub struct NullTagger;
impl WordTagger for NullTagger {
    fn is_tagged(&self, _word: &str) -> bool {
        false
    }
}

// Base WordTokenizer.TOKENIZING_CHARACTERS + English's extra '_'. Only used by
// the URL/whitespace helpers below (the main split uses `tokenizer_pattern`).
const PROTOCOLS: [&str; 3] = ["http", "https", "ftp"];

// EnglishWordTokenizer.wordCharacters, with Java `\uXXXX` rewritten `\x{XXXX}`.
const WORD_CHARACTERS: &str =
    r"±§©@€£¥\$\p{L}\d\-\x{0300}-\x{036F}\x{00A8}°%‰‱&\x{FFFD}\x{00AD}\x{00AC}\x{FF0C}\x{FF1F}";

fn tokenizer_pattern() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| {
        Regex::new(&format!("[{WORD_CHARACTERS}]+|[^{WORD_CHARACTERS}]")).unwrap()
    })
}

fn contraction_patterns() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        // (?i) == CASE_INSENSITIVE | UNICODE_CASE. `'` and `'` are the straight
        // and curly apostrophes LT lists explicitly.
        [
            r"(?i)^(fo['']c['']sle|rec[''][ds]|OK['']d|cc[''][ds]|DJ['']d|[pd]m['']d|rsvp['']d)$",
            r"(?i)^(['']?)(are|is|were|was|do|does|did|have|has|had|wo|would|ca|could|sha|should|must|ai|ought|might|need|may|am|dare|das|dass|hai|used|use)(n['']t)$",
            r"(?i)^(.+)(['']m|['']re|['']ll|['']ve|['']d|['']s)([''-]?)$",
            r"(?i)^(['']t)(was|were|is)$",
        ]
        .iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    })
}

fn url_chars() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    // NB: `$-_` is a RANGE (U+0024..U+005F) in LT's Java pattern — it covers `:`,
    // `;`, `=`, `?`, `@`, digits, uppercase, etc. Do not escape the hyphen.
    P.get_or_init(|| Regex::new(r"^[a-zA-ZÄÖÜäöü0-9/%$-_.+!*'(),?#~]+$").unwrap())
}
fn domain_chars() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9-]+$").unwrap())
}
fn no_protocol_url() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| {
        Regex::new(r"^([a-zA-Z0-9][a-zA-Z0-9-]+\.)?([a-zA-Z0-9][a-zA-Z0-9-]+)\.([a-zA-Z0-9][a-zA-Z0-9-]+)/.*$").unwrap()
    })
}
fn e_mail() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    // Java \b at the ends; fancy-regex supports it. Used with find (not anchored).
    P.get_or_init(|| {
        Regex::new(r"(?<!:)@?\b[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@((\[[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\])|(([a-zA-Z\-0-9]+\.)+[a-zA-Z]{2,}))\b").unwrap()
    })
}

const APOSTYPEW: &str = "xxAPOSTYPEWxx";
const APOSTYPOG: &str = "xxAPOSTYPOGxx";

pub struct EnglishWordTokenizer<T: WordTagger> {
    tagger: T,
}

impl<T: WordTagger> EnglishWordTokenizer<T> {
    pub fn new(tagger: T) -> Self {
        EnglishWordTokenizer { tagger }
    }

    /// Tokenize `text`, returning words, whitespace, and punctuation as separate
    /// tokens (concatenation reproduces `text`), matching LT.
    pub fn tokenize(&self, text: &str) -> Vec<String> {
        // Shield apostrophes so they stay inside word-character runs.
        let aux = text.replace('\'', APOSTYPEW).replace('\u{2019}', APOSTYPOG);

        let mut l: Vec<String> = Vec::new();
        let re = tokenizer_pattern();
        let mut it = re.find_iter(&aux);
        while let Some(Ok(m)) = it.next() {
            let raw = m.as_str();
            // Variation selectors attach to the previous token.
            if !l.is_empty() && raw.chars().count() == 1 {
                let cp = raw.chars().next().unwrap() as u32;
                if (0xFE00..=0xFE0F).contains(&cp) {
                    let last = l.last_mut().unwrap();
                    last.push_str(raw);
                    continue;
                }
            }
            let s = raw.replace(APOSTYPEW, "'").replace(APOSTYPOG, "\u{2019}");

            let mut matched: Option<Vec<String>> = None;
            if s.contains('\'') || s.contains('\u{2019}') {
                for pat in contraction_patterns() {
                    if let Ok(Some(caps)) = pat.captures(&s) {
                        let mut groups = Vec::new();
                        for i in 1..caps.len() {
                            if let Some(g) = caps.get(i) {
                                groups.extend(self.words_to_add(g.as_str()));
                            }
                        }
                        matched = Some(groups);
                        break;
                    }
                }
            }
            match matched {
                Some(groups) => l.extend(groups),
                None => l.extend(self.words_to_add(&s)),
            }
        }
        join_emails_and_urls(l)
    }

    /// Port of `wordsToAdd`: strip leading/trailing hyphens as their own tokens,
    /// keep a `-`/`'`-containing word whole if the tagger knows it (or it's in
    /// LT's hard-coded allowlist), else split it on apostrophes.
    fn words_to_add(&self, input: &str) -> Vec<String> {
        let mut l = Vec::new();
        if input.is_empty() {
            return l;
        }
        let mut s = input.to_string();
        while let Some(rest) = s.strip_prefix('-') {
            l.push("-".to_string());
            s = rest.to_string();
        }
        let mut hyphens_at_end = 0;
        while let Some(rest) = s.strip_suffix('-') {
            s = rest.to_string();
            hyphens_at_end += 1;
        }
        if !s.is_empty() {
            let has_marks = s.contains('-') || s.contains('\'') || s.contains('\u{2019}');
            if !has_marks {
                l.push(s.clone());
            } else {
                let normalized = s.replace('\u{00AD}', "").replace('\u{2019}', "'");
                if self.tagger.is_tagged(&normalized) || is_allowlisted(&s) {
                    l.push(s.clone());
                } else {
                    // StringTokenizer(s, "''", true): split on either apostrophe,
                    // returning the delimiters as their own tokens.
                    for tok in split_keep_delims(&s, &['\'', '\u{2019}']) {
                        l.push(tok);
                    }
                }
            }
        }
        for _ in 0..hyphens_at_end {
            l.push("-".to_string());
        }
        l
    }
}

fn is_allowlisted(s: &str) -> bool {
    const LIST: [&str; 10] = [
        "mers-cov", "mcgraw-hill", "sars-cov-2", "sars-cov", "ph-metre", "ph-metres",
        "anti-ivg", "anti-uv", "anti-vih", "al-qaida",
    ];
    LIST.iter().any(|w| w.eq_ignore_ascii_case(s))
}

/// Java `StringTokenizer(s, delims, returnDelims=true)`: maximal non-delimiter
/// runs and each delimiter char, in order.
fn split_keep_delims(s: &str, delims: &[char]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if delims.contains(&ch) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(ch.to_string());
        } else {
            cur.push(ch);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---- URL / e-mail joining (base WordTokenizer) ----

fn join_emails_and_urls(list: Vec<String>) -> Vec<String> {
    join_urls(join_emails(list))
}

fn is_whitespace(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| c.is_whitespace())
}

fn join_emails(list: Vec<String>) -> Vec<String> {
    let text: String = list.concat();
    if !text.contains('@') {
        return list;
    }
    let re = e_mail();
    if !matches!(re.is_match(&text), Ok(true)) {
        return list;
    }
    let mut l: Vec<String> = Vec::new();
    let mut current = 0usize; // byte position
    let mut idx = 0usize;
    let mut it = re.find_iter(&text);
    while let Some(Ok(m)) = it.next() {
        let (start, end) = (m.start(), m.end());
        while current < end && idx < list.len() {
            if current < start {
                l.push(list[idx].clone());
            } else if current == start {
                l.push(m.as_str().to_string());
            }
            current += list[idx].len();
            idx += 1;
        }
    }
    if idx < list.len() {
        l.extend_from_slice(&list[idx..]);
    }
    l
}

fn is_protocol(tok: &str) -> bool {
    PROTOCOLS.contains(&tok)
}

fn domain_matches(tok: &str) -> bool {
    matches!(domain_chars().is_match(tok), Ok(true))
}

fn url_starts_at(i: usize, l: &[String]) -> bool {
    let token = &l[i];
    if is_protocol(token) && l.len() > i + 3 && l[i + 1] == ":" && l[i + 2] == "/" && l[i + 3] == "/"
    {
        return true;
    }
    if l.len() > i + 1 && l[i] == "www" && l[i + 1] == "." {
        return true;
    }
    if l.len() > i + 3
        && l[i + 1] == "."
        && l[i + 3] == "/"
        && domain_matches(token)
        && domain_matches(&l[i + 2])
    {
        return true;
    }
    l.len() > i + 5
        && l[i + 1] == "."
        && l[i + 3] == "."
        && l[i + 5] == "/"
        && domain_matches(token)
        && domain_matches(&l[i + 2])
        && domain_matches(&l[i + 4])
}

fn url_ends_at(i: usize, l: &[String], url_quote: Option<&str>) -> bool {
    let token = &l[i];
    if is_whitespace(token) || token == ")" || token == "]" {
        return true;
    }
    if l.len() > i + 1 {
        let next = &l[i + 1];
        const CLOSERS: [&str; 9] =
            ["\"", "»", "«", "'", "\u{2019}", "\u{201C}", "\u{201D}", "\u{2018}", "."];
        const ENDERS: [&str; 6] = [".", ",", ";", ":", "!", "?"];
        let next_closer = is_whitespace(next) || CLOSERS.contains(&next.as_str());
        let tok_ender = ENDERS.contains(&token.as_str()) || Some(token.as_str()) == url_quote;
        if (next_closer && tok_ender) || !matches!(url_chars().is_match(token), Ok(true)) {
            return true;
        }
    } else {
        if !matches!(url_chars().is_match(token), Ok(true))
            || token == "."
            || Some(token.as_str()) == url_quote
        {
            return true;
        }
    }
    false
}

fn join_urls(l: Vec<String>) -> Vec<String> {
    let mut new_list: Vec<String> = Vec::new();
    let mut in_url = false;
    let mut url = String::new();
    let mut url_quote: Option<String> = None;
    for i in 0..l.len() {
        if url_starts_at(i, &l) && !in_url {
            in_url = true;
            if i >= 1 {
                url_quote = Some(l[i - 1].clone());
            }
            url.push_str(&l[i]);
        } else if in_url && url_ends_at(i, &l, url_quote.as_deref()) {
            in_url = false;
            url_quote = None;
            new_list.push(std::mem::take(&mut url));
            new_list.push(l[i].clone());
        } else if in_url {
            url.push_str(&l[i]);
        } else {
            new_list.push(l[i].clone());
        }
    }
    if !url.is_empty() {
        new_list.push(url);
    }
    new_list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub tagger recognising the tokens LT's real tagger would, so we can
    /// verify the tagger-coupled paths before Phase 1.3 exists.
    struct StubTagger;
    impl WordTagger for StubTagger {
        fn is_tagged(&self, w: &str) -> bool {
            matches!(
                w.to_lowercase().as_str(),
                "n't" | "o'clock" | "'s" | "'re" | "'ve" | "'ll" | "'d" | "'m"
            )
        }
    }

    fn toks(text: &str) -> Vec<String> {
        EnglishWordTokenizer::new(StubTagger).tokenize(text)
    }

    #[test]
    fn splits_punctuation_and_spaces() {
        assert_eq!(
            toks("Hello world."),
            vec!["Hello", " ", "world", "."]
        );
    }

    #[test]
    fn contraction_nt_kept_whole() {
        // matches LT oracle: don't -> [do, n't]
        assert_eq!(toks("don't"), vec!["do", "n't"]);
        assert_eq!(toks("I don't know"),
            vec!["I", " ", "do", "n't", " ", "know"]);
    }

    #[test]
    fn keeps_url_and_email_whole() {
        assert_eq!(toks("Visit www.example.com."),
            vec!["Visit", " ", "www.example.com", "."]);
        assert_eq!(toks("a b@c.com d"),
            vec!["a", " ", "b@c.com", " ", "d"]);
    }

    #[test]
    fn trailing_hyphen_is_own_token() {
        assert_eq!(toks("well- being"), vec!["well", "-", " ", "being"]);
    }
}
