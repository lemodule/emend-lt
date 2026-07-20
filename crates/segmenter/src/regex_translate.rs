//! Translate the Java-regex dialect used in SRX rule files into the syntax the
//! `fancy-regex` crate accepts.
//!
//! The SRX regexes (`useJavaRegex="yes"`) are almost entirely compatible with
//! `fancy-regex`: `\p{Lu}`, `\b`, `(?i)`, lookarounds, quantifiers all carry over
//! unchanged. The one systematic difference is the Unicode escape: Java writes
//! ` `, whereas fancy-regex wants `\x{00A0}`.

/// Rewrite `\uXXXX` (exactly four hex digits) as `\x{XXXX}`. A backslash that is
/// itself escaped (`\\u`) is left alone.
pub fn java_to_fancy(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Count the run of backslashes; only an odd run introduces an escape.
            let start = i;
            while i < bytes.len() && bytes[i] == b'\\' {
                i += 1;
            }
            let run = i - start;
            let escaping = run % 2 == 1;
            if escaping && i < bytes.len() && bytes[i] == b'u' && has_4_hex(bytes, i + 1) {
                // Emit all but the last backslash, then the translated escape.
                for _ in 0..run - 1 {
                    out.push('\\');
                }
                let hex = &src[i + 1..i + 5];
                out.push_str("\\x{");
                out.push_str(hex);
                out.push('}');
                i += 5; // consumed 'u' + 4 hex
            } else {
                for _ in 0..run {
                    out.push('\\');
                }
            }
        } else {
            // Copy one UTF-8 char.
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn has_4_hex(bytes: &[u8], at: usize) -> bool {
    at + 4 <= bytes.len() && bytes[at..at + 4].iter().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::java_to_fancy;

    #[test]
    fn translates_unicode_escape() {
        assert_eq!(java_to_fancy(r"[\s\u00A0]"), r"[\s\x{00A0}]");
        assert_eq!(java_to_fancy(r"\u2029"), r"\x{2029}");
    }

    #[test]
    fn leaves_other_escapes_alone() {
        assert_eq!(java_to_fancy(r"\b[Ee]tc\."), r"\b[Ee]tc\.");
        assert_eq!(java_to_fancy(r"\p{Lu}\p{Ll}+"), r"\p{Lu}\p{Ll}+");
    }

    #[test]
    fn respects_escaped_backslash() {
        // \\u is a literal backslash then 'u', not a unicode escape.
        assert_eq!(java_to_fancy(r"\\u00A0"), r"\\u00A0");
    }
}
