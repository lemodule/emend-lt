//! LanguageTool rule files declare regex-fragment macros in their prolog via
//! `<!ENTITY name "value">` and reference them as `&name;` inside token text and
//! attributes (e.g. `&uncommon_verbs;`, `&apostrophe;`). quick-xml does not
//! expand custom entities, so the whole file must be pre-expanded before pattern
//! parsing. Standard XML entities (`&amp;`, `&lt;`, …) are left untouched.

use std::collections::HashMap;

/// Extract `<!ENTITY name "value">` declarations from `xml`.
pub fn parse_entity_defs(xml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    let needle = b"<!ENTITY";
    while let Some(p) = find(bytes, i, needle) {
        let mut j = p + needle.len();
        // name
        j = skip_ws(bytes, j);
        let name_start = j;
        while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let name = &xml[name_start..j];
        j = skip_ws(bytes, j);
        // quoted value
        if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
            let quote = bytes[j];
            j += 1;
            let val_start = j;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            let value = &xml[val_start..j];
            if !name.is_empty() {
                map.insert(name.to_string(), value.to_string());
            }
            j += 1;
        }
        i = j;
    }
    map
}

/// Replace every `&name;` reference (for a declared custom entity) with its
/// value, expanding nested references up to a fixed depth. Standard XML
/// entities and unknown `&...;` are left as-is.
pub fn expand(xml: &str, defs: &HashMap<String, String>) -> String {
    if defs.is_empty() {
        return xml.to_string();
    }
    let mut cur = xml.to_string();
    for _ in 0..8 {
        let (next, changed) = expand_once(&cur, defs);
        cur = next;
        if !changed {
            break;
        }
    }
    cur
}

fn expand_once(xml: &str, defs: &HashMap<String, String>) -> (String, bool) {
    let mut out = String::with_capacity(xml.len());
    let bytes = xml.as_bytes();
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi) = bytes[i + 1..].iter().position(|&b| b == b';') {
                let name = &xml[i + 1..i + 1 + semi];
                if let Some(val) = defs.get(name) {
                    out.push_str(val);
                    i += 1 + semi + 1;
                    changed = true;
                    continue;
                }
            }
        }
        let ch = xml[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, changed)
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn find(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_and_expands() {
        let xml = r#"<!ENTITY apostrophe "['’]">
                     <!ENTITY greet 'hi|hey'>
                     <pattern><token regexp="yes">&greet;&apostrophe;</token></pattern>"#;
        let defs = parse_entity_defs(xml);
        assert_eq!(defs.get("apostrophe").unwrap(), "['’]");
        assert_eq!(defs.get("greet").unwrap(), "hi|hey");
        let expanded = expand(xml, &defs);
        assert!(expanded.contains("hi|hey['’]"));
        assert!(!expanded.contains("&greet;"));
    }

    #[test]
    fn leaves_standard_entities() {
        let defs = parse_entity_defs("<!ENTITY x \"y\">");
        let out = expand("a &amp; b &x;", &defs);
        assert_eq!(out, "a &amp; b y");
    }
}
