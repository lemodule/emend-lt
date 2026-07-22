//! The loaded checking engine: owns every data structure the pipeline needs
//! (tagger, disambiguation rules, grammar rules, sentence segmenter, optional
//! synthesizer, optional spelling rule) and turns a whole text into the merged
//! `matches[]` that `/v2/check` returns.
//!
//! This is the server-side counterpart of the `check-text` / `spell-text`
//! binaries: it performs the exact same per-sentence pipeline (segment → raw
//! analyze → disambiguate → grammar rules (+ synthesizer) → spelling rule) but
//! loads once and runs many times.

use analysis::grammar::{check_sentence, parse_grammar_rules};
use analysis::{parse_disambig_rules, Analyzer, DisambigRule, GrammarMatch, GrammarRule, SpellRule};
use matcher::{expand, parse_entity_defs};
use morfologik::Synthesizer;
use std::path::Path;
use tagger::EnglishTagger;

/// Paths to every data file the engine needs. Spelling and synthesis are
/// optional (leave `None` to skip that subsystem).
pub struct EngineConfig {
    /// `english.dict` (the tagger/POS dictionary). `added.txt`/`removed.txt` and
    /// the `.info` file are read from alongside it.
    pub dict: String,
    pub disambiguation_xml: String,
    pub grammar_xml: String,
    pub srx: String,
    /// `english_synth.dict` + `english_tags.txt` (both or neither).
    pub synth_dict: Option<String>,
    pub synth_tags: Option<String>,
    /// `en_US.dict` (Hunspell spelling FSA). Enables `MORFOLOGIK_RULE_EN_US`.
    pub spell_dict: Option<String>,
    /// `ignore.txt` + `spelling*.txt` accept-list bodies.
    pub spell_ignore: Vec<String>,
    /// `multiwords.txt`.
    pub spell_multiwords: Option<String>,
    /// `prohibit.txt`.
    pub spell_prohibit: Option<String>,
}

pub struct Engine {
    tagger: EnglishTagger,
    disambig: Vec<DisambigRule>,
    grammar: Vec<GrammarRule>,
    segmenter: segmenter::Segmenter,
    synth: Option<Synthesizer>,
    spell: Option<SpellRule>,
}

impl Engine {
    /// Load every data file and build the engine. Returns a human-readable error
    /// string on any I/O or parse failure.
    pub fn load(cfg: &EngineConfig) -> Result<Engine, String> {
        let dictp = Path::new(&cfg.dict);
        let base = dictp.parent().unwrap_or_else(|| Path::new(""));
        let info = read_sibling(&cfg.dict, ".dict", ".info")?;
        let db = read_bytes(&cfg.dict)?;
        let added = read_opt(&base.join("added.txt"));
        let removed = read_opt(&base.join("removed.txt"));
        let tagger = EnglishTagger::load_with_manual(&db, &info, &added, &removed)
            .map_err(|e| format!("tagger load: {e:?}"))?;

        let dx = read_text(&cfg.disambiguation_xml)?;
        let dd = parse_entity_defs(&dx);
        let disambig = parse_disambig_rules(&expand(&dx, &dd))
            .map_err(|e| format!("disambiguation parse: {e}"))?;

        let gx = read_text(&cfg.grammar_xml)?;
        let gd = parse_entity_defs(&gx);
        let grammar =
            parse_grammar_rules(&expand(&gx, &gd)).map_err(|e| format!("grammar parse: {e}"))?;

        let sx = read_text(&cfg.srx)?;
        let segmenter =
            segmenter::Segmenter::from_srx(&sx, "en-US").map_err(|e| format!("srx parse: {e}"))?;

        let synth = match (&cfg.synth_dict, &cfg.synth_tags) {
            (Some(sd), Some(st)) => {
                let sdict = read_bytes(sd)?;
                let sinfo = read_sibling(sd, ".dict", ".info")?;
                let stags = read_text(st)?;
                Some(
                    Synthesizer::load(&sdict, &sinfo, &stags)
                        .map_err(|e| format!("synthesizer load: {e:?}"))?,
                )
            }
            _ => None,
        };

        let spell = match &cfg.spell_dict {
            Some(sp) => {
                let spdb = read_bytes(sp)?;
                let spinfo = read_sibling(sp, ".dict", ".info")?;
                let ignore_texts: Vec<String> = cfg
                    .spell_ignore
                    .iter()
                    .map(|p| read_opt(Path::new(p)))
                    .collect();
                let ignore_refs: Vec<&str> = ignore_texts.iter().map(|s| s.as_str()).collect();
                let multiwords = cfg
                    .spell_multiwords
                    .as_ref()
                    .map(|p| read_opt(Path::new(p)))
                    .unwrap_or_default();
                let prohibit = cfg
                    .spell_prohibit
                    .as_ref()
                    .map(|p| read_opt(Path::new(p)))
                    .unwrap_or_default();
                Some(
                    SpellRule::load(&spdb, &spinfo, &ignore_refs, &multiwords, &prohibit)
                        .map_err(|e| format!("spell rule load: {e:?}"))?,
                )
            }
            None => None,
        };

        Ok(Engine {
            tagger,
            disambig,
            grammar,
            segmenter,
            synth,
            spell,
        })
    }

    /// Run the full pipeline over `text`, returning the merged, sorted matches.
    /// `enabled` decides, per rule/category id, whether a match is kept (used to
    /// honor `disabledRules`/`disabledCategories`).
    pub fn check(&self, text: &str, keep: impl Fn(&str, &str) -> bool) -> Vec<GrammarMatch> {
        let analyzer = Analyzer::new(&self.tagger);
        let synth_ref = self.synth.as_ref();
        let mut out: Vec<GrammarMatch> = Vec::new();
        let mut char_off = 0usize;
        for s in self.segmenter.segment(text) {
            let mut t = analyzer.raw(&s);
            analysis::disambig::apply_all(&self.disambig, &mut t);
            out.extend(check_sentence(&self.grammar, &t, &s, char_off, synth_ref));
            if let Some(spell) = &self.spell {
                out.extend(spell.check_sentence(&t, &s, char_off));
            }
            char_off += s.chars().count();
        }
        out.retain(|m| keep(&m.rule_id, &m.category_id));
        // LT returns matches in document order.
        out.sort_by(|a, b| a.offset.cmp(&b.offset).then(b.length.cmp(&a.length)));
        out
    }
}

fn read_bytes(p: &str) -> Result<Vec<u8>, String> {
    std::fs::read(p).map_err(|e| format!("read {p}: {e}"))
}

fn read_text(p: &str) -> Result<String, String> {
    std::fs::read_to_string(p).map_err(|e| format!("read {p}: {e}"))
}

fn read_opt(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

/// Read a file whose path is `base_path` with `from` swapped for `to`
/// (e.g. `english.dict` → `english.info`).
fn read_sibling(base_path: &str, from: &str, to: &str) -> Result<String, String> {
    let sib = base_path
        .strip_suffix(from)
        .map(|s| format!("{s}{to}"))
        .ok_or_else(|| format!("{base_path} does not end in {from}"))?;
    Ok(std::fs::read_to_string(&sib).unwrap_or_default())
}
