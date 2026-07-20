//! Minimal SRX (Segmentation Rules eXchange) parser — enough of the format to
//! reproduce LanguageTool's `segment.srx`: `<languagerules>` with ordered
//! break/no-break `<rule>`s, and `<maprules>` mapping a language code to one or
//! more rule sets (with `cascade="yes"`, every matching map contributes).

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone)]
pub struct SrxRule {
    pub is_break: bool,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone)]
struct LanguageMap {
    pattern: String,
    rule_name: String,
}

#[derive(Debug)]
pub struct SrxDocument {
    cascade: bool,
    rules_by_name: Vec<(String, Vec<SrxRule>)>,
    maps: Vec<LanguageMap>,
}

impl SrxDocument {
    pub fn parse(xml: &str) -> Result<SrxDocument, String> {
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(false);

        let mut cascade = false;
        let mut rules_by_name: Vec<(String, Vec<SrxRule>)> = Vec::new();
        let mut maps: Vec<LanguageMap> = Vec::new();

        let mut cur_rule_name: Option<String> = None;
        let mut cur_rules: Vec<SrxRule> = Vec::new();
        let mut cur_break = true;
        let mut cur_before = String::new();
        let mut cur_after = String::new();
        // Which element's text we are currently accumulating.
        let mut in_before = false;
        let mut in_after = false;

        loop {
            match reader.read_event() {
                Err(e) => return Err(format!("XML error at {}: {e}", reader.error_position())),
                Ok(Event::Eof) => break,
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let name = e.local_name();
                    match name.as_ref() {
                        b"header" => {
                            for a in e.attributes().flatten() {
                                if a.key.local_name().as_ref() == b"cascade" {
                                    cascade = a.unescape_value().unwrap_or_default() == "yes";
                                }
                            }
                        }
                        b"languagerule" => {
                            for a in e.attributes().flatten() {
                                if a.key.local_name().as_ref() == b"languagerulename" {
                                    cur_rule_name =
                                        Some(a.unescape_value().unwrap_or_default().into_owned());
                                }
                            }
                            cur_rules = Vec::new();
                        }
                        b"languagemap" => {
                            let mut pattern = String::new();
                            let mut rule_name = String::new();
                            for a in e.attributes().flatten() {
                                match a.key.local_name().as_ref() {
                                    b"languagepattern" => {
                                        pattern = a.unescape_value().unwrap_or_default().into_owned()
                                    }
                                    b"languagerulename" => {
                                        rule_name =
                                            a.unescape_value().unwrap_or_default().into_owned()
                                    }
                                    _ => {}
                                }
                            }
                            maps.push(LanguageMap { pattern, rule_name });
                        }
                        b"rule" => {
                            cur_break = true;
                            cur_before.clear();
                            cur_after.clear();
                            for a in e.attributes().flatten() {
                                if a.key.local_name().as_ref() == b"break" {
                                    cur_break = a.unescape_value().unwrap_or_default() != "no";
                                }
                            }
                        }
                        b"beforebreak" => in_before = true,
                        b"afterbreak" => in_after = true,
                        _ => {}
                    }
                }
                Ok(Event::Text(t)) => {
                    if in_before || in_after {
                        let raw = String::from_utf8_lossy(t.as_ref()).into_owned();
                        let txt = quick_xml::escape::unescape(&raw)
                            .map(|c| c.into_owned())
                            .unwrap_or(raw);
                        if in_before {
                            cur_before.push_str(&txt);
                        } else {
                            cur_after.push_str(&txt);
                        }
                    }
                }
                Ok(Event::End(e)) => match e.local_name().as_ref() {
                    b"beforebreak" => in_before = false,
                    b"afterbreak" => in_after = false,
                    b"rule" => cur_rules.push(SrxRule {
                        is_break: cur_break,
                        before: cur_before.clone(),
                        after: cur_after.clone(),
                    }),
                    b"languagerule" => {
                        if let Some(n) = cur_rule_name.take() {
                            rules_by_name.push((n, std::mem::take(&mut cur_rules)));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(SrxDocument {
            cascade,
            rules_by_name,
            maps,
        })
    }

    /// The ordered, concatenated rule list that applies to `lang_code`. With
    /// cascade, every `<languagemap>` whose pattern fully matches contributes its
    /// rules, in document order; without cascade only the first match applies.
    pub fn rules_for(&self, lang_code: &str) -> Vec<SrxRule> {
        let mut out = Vec::new();
        for m in &self.maps {
            if full_match(&m.pattern, lang_code) {
                if let Some((_, rules)) = self.rules_by_name.iter().find(|(n, _)| *n == m.rule_name)
                {
                    out.extend(rules.iter().cloned());
                }
                if !self.cascade {
                    break;
                }
            }
        }
        out
    }
}

/// Java `String.matches` semantics: the whole string must match the pattern.
fn full_match(pattern: &str, text: &str) -> bool {
    let anchored = format!("^(?:{pattern})$");
    match fancy_regex::Regex::new(&anchored) {
        Ok(re) => re.is_match(text).unwrap_or(false),
        Err(_) => false,
    }
}
