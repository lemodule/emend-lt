//! Dictionary layer on top of the raw CFSA2 automaton: `.info` metadata parsing,
//! word membership (spelling), and SUFFIX-encoded stem/tag decoding (POS).

use crate::fsa::Cfsa2;

/// The base-form encoder declared by `fsa.dict.encoder`. Only SUFFIX is
/// implemented so far (it is what the English dictionaries use); the others are
/// stubbed so an unexpected dictionary fails loudly rather than silently wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// No relative encoding — the field after the separator is the literal base.
    None,
    /// Trim N trailing bytes from the inflected form, then append a suffix.
    Suffix,
    Prefix,
    Infix,
}

impl Encoder {
    fn parse(s: &str) -> Encoder {
        match s.trim() {
            "SUFFIX" => Encoder::Suffix,
            "PREFIX" => Encoder::Prefix,
            "INFIX" => Encoder::Infix,
            _ => Encoder::None,
        }
    }
}

/// A decoded stemming entry: the base (lemma) and its POS tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictEntry {
    pub stem: String,
    pub tag: String,
}

pub struct Dictionary {
    fsa: Cfsa2,
    separator: u8,
    encoder: Encoder,
    frequency_included: bool,
}

impl Dictionary {
    /// Load from the raw `.dict` bytes plus the text of the sibling `.info` file.
    pub fn load(dict_bytes: &[u8], info_text: &str) -> Result<Dictionary, crate::fsa::FsaError> {
        let mut separator = b'+';
        let mut encoder = Encoder::None;
        let mut frequency_included = false;
        for line in info_text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else { continue };
            match key.trim() {
                "fsa.dict.separator" => {
                    if let Some(c) = val.trim().bytes().next() {
                        separator = c;
                    }
                }
                "fsa.dict.encoder" => encoder = Encoder::parse(val),
                "fsa.dict.frequency-included" => {
                    frequency_included = val.trim().eq_ignore_ascii_case("true")
                }
                _ => {}
            }
        }
        let fsa = Cfsa2::parse(dict_bytes)?;
        Ok(Dictionary {
            fsa,
            separator,
            encoder,
            frequency_included,
        })
    }

    pub fn separator(&self) -> u8 {
        self.separator
    }

    pub fn encoder(&self) -> Encoder {
        self.encoder
    }

    pub fn fsa(&self) -> &Cfsa2 {
        &self.fsa
    }

    /// Follow `bytes` from the root node. Returns the destination node reached
    /// after consuming the last byte, plus whether that last arc was final.
    /// Returns None if the path does not exist.
    fn walk_path(&self, bytes: &[u8]) -> Option<(usize, bool)> {
        let mut node = self.fsa.root_node();
        let mut last_final = false;
        for (i, &b) in bytes.iter().enumerate() {
            let arc = self.fsa.get_arc(node, b);
            if arc == 0 {
                return None;
            }
            last_final = self.fsa.is_arc_final(arc);
            let last = i + 1 == bytes.len();
            if !last {
                if self.fsa.is_arc_terminal(arc) {
                    return None;
                }
                node = self.fsa.get_end_node(arc);
            } else if !self.fsa.is_arc_terminal(arc) {
                node = self.fsa.get_end_node(arc);
            }
        }
        Some((node, last_final))
    }

    /// Spelling membership: is `word` present in the dictionary?
    ///
    /// Spelling dictionaries store each word optionally followed by a
    /// `separator` + frequency byte. So a word is "known" if either the path
    /// spelling it ends on a final arc, or there is a `separator` arc out of the
    /// node it reaches (the frequency tail).
    pub fn contains(&self, word: &str) -> bool {
        let bytes = word.as_bytes();
        let Some((node, last_final)) = self.walk_path(bytes) else {
            return false;
        };
        if !self.frequency_included && last_final {
            return true;
        }
        // A separator arc out of the reached node marks a complete entry
        // (frequency tail or stem/tag tail).
        self.fsa.get_arc(node, self.separator) != 0 || last_final
    }

    /// POS lookup: decode every `(stem, tag)` entry stored for `word`.
    ///
    /// Stemming entries are stored as `inflected SEP encodedStem SEP tag`. We
    /// walk `inflected`, then collect each tail after the separator, split it on
    /// the separator again into encoded-stem and tag, and decode the stem.
    pub fn lookup(&self, word: &str) -> Vec<DictEntry> {
        let bytes = word.as_bytes();
        let Some((node, _)) = self.walk_path(bytes) else {
            return Vec::new();
        };
        let sep_arc = self.fsa.get_arc(node, self.separator);
        if sep_arc == 0 || self.fsa.is_arc_terminal(sep_arc) {
            return Vec::new();
        }
        // Collect all tails (byte sequences) reachable after the separator arc.
        let tail_start = self.fsa.get_end_node(sep_arc);
        let mut tails: Vec<Vec<u8>> = Vec::new();
        collect_sequences(&self.fsa, tail_start, &mut Vec::new(), &mut tails);

        let mut out = Vec::new();
        for tail in tails {
            // tail = encodedStem SEP tag
            let (enc_stem, tag) = match split_once(&tail, self.separator) {
                Some((a, b)) => (a, b),
                None => (&tail[..], &b""[..]),
            };
            let stem = self.decode_stem(bytes, enc_stem);
            out.push(DictEntry {
                stem: String::from_utf8_lossy(&stem).into_owned(),
                tag: String::from_utf8_lossy(tag).into_owned(),
            });
        }
        out
    }

    /// Reconstruct the base form from the inflected form and the encoded field,
    /// per the declared encoder. SUFFIX: first byte is `('A' + trimCount)`; drop
    /// that many bytes from the end of `inflected`, then append the remainder.
    fn decode_stem(&self, inflected: &[u8], encoded: &[u8]) -> Vec<u8> {
        match self.encoder {
            Encoder::Suffix => decode_suffix(inflected, encoded),
            Encoder::None => encoded.to_vec(),
            // Not needed for English; implement when a dictionary requires it.
            Encoder::Prefix | Encoder::Infix => encoded.to_vec(),
        }
    }
}

/// SUFFIX decode: the first encoded byte is `('A' + trimCount)`; drop that many
/// bytes from the end of `inflected`, then append the rest of `encoded`.
fn decode_suffix(inflected: &[u8], encoded: &[u8]) -> Vec<u8> {
    if encoded.is_empty() {
        return inflected.to_vec();
    }
    let trim = encoded[0].wrapping_sub(b'A') as usize;
    let keep = inflected.len().saturating_sub(trim);
    let mut stem = inflected[..keep].to_vec();
    stem.extend_from_slice(&encoded[1..]);
    stem
}

#[cfg(test)]
mod tests {
    use super::decode_suffix;

    #[test]
    fn suffix_regular_inflection() {
        // walked+C+VBD : trim 'C'-'A'=2 -> "walk"
        assert_eq!(decode_suffix(b"walked", b"C"), b"walk");
        // houses+B... : trim 1 -> "house"
        assert_eq!(decode_suffix(b"houses", b"B"), b"house");
    }

    #[test]
    fn suffix_irregular_with_appended_suffix() {
        // mice -> mouse : trim all 4, append "mouse"  (code 'E' = 4)
        assert_eq!(decode_suffix(b"mice", b"Emouse"), b"mouse");
        // better -> good : trim 6, append "good"  (code 'G' = 6)
        assert_eq!(decode_suffix(b"better", b"Ggood"), b"good");
    }

    #[test]
    fn suffix_identity() {
        // code 'A' = 0 trimmed, nothing appended -> unchanged
        assert_eq!(decode_suffix(b"running", b"A"), b"running");
    }
}

fn split_once(bytes: &[u8], sep: u8) -> Option<(&[u8], &[u8])> {
    bytes.iter().position(|&b| b == sep).map(|i| (&bytes[..i], &bytes[i + 1..]))
}

/// Depth-first collect of every accepted sequence rooted at `node`.
fn collect_sequences(fsa: &Cfsa2, node: usize, buf: &mut Vec<u8>, out: &mut Vec<Vec<u8>>) {
    let mut arc = fsa.get_first_arc(node);
    while arc != 0 {
        buf.push(fsa.get_arc_label(arc));
        if fsa.is_arc_final(arc) {
            out.push(buf.clone());
        }
        if !fsa.is_arc_terminal(arc) {
            let next = fsa.get_end_node(arc);
            collect_sequences(fsa, next, buf, out);
        }
        buf.pop();
        arc = fsa.get_next_arc(arc);
    }
}
