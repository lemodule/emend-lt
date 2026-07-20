//! Analyze pre-segmented sentences (one per non-empty line) and dump tokens in
//! the same format as the Java `AnalyzedOracle`, for direct diffing.
//!
//! Usage: analyze <raw|dis> <english.dict> <disambiguation.xml> <probes.txt>

use std::process::ExitCode;

use analysis::{parse_disambig_rules, Analyzer};
use matcher::{expand, parse_entity_defs};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!(
            "usage: analyze <raw|dis> <english.dict> <disambiguation.xml> <probes.txt> [segment.srx]"
        );
        return ExitCode::FAILURE;
    }
    let mode = &args[1];
    let dict_path = &args[2];
    let disambig_path = &args[3];
    let probes_path = &args[4];
    let srx_path = args.get(5);

    // Optional sentence segmentation (verified byte-exact in phase 1.2); when a
    // segment.srx is given, split each probe line into sentences to match the
    // Java oracle's sentence tokenizer.
    let segmenter = srx_path.map(|p| {
        let xml = std::fs::read_to_string(p).expect("read segment.srx");
        segmenter::Segmenter::from_srx(&xml, "en-US").expect("build segmenter")
    });

    let base = dict_path.strip_suffix("english.dict").unwrap_or("");
    let info_path = dict_path
        .strip_suffix(".dict")
        .map(|s| format!("{s}.info"))
        .unwrap_or_default();
    let dict_bytes = std::fs::read(dict_path).expect("read dict");
    let info = std::fs::read_to_string(&info_path).unwrap_or_default();
    let added = std::fs::read_to_string(format!("{base}added.txt")).unwrap_or_default();
    let removed = std::fs::read_to_string(format!("{base}removed.txt")).unwrap_or_default();
    let tagger =
        tagger::EnglishTagger::load_with_manual(&dict_bytes, &info, &added, &removed).unwrap();
    let analyzer = Analyzer::new(&tagger);

    let disambiguate = mode != "raw";
    let rules = if disambiguate {
        let xml = std::fs::read_to_string(disambig_path).expect("read disambiguation.xml");
        let defs = parse_entity_defs(&xml);
        let expanded = expand(&xml, &defs);
        parse_disambig_rules(&expanded).expect("parse disambiguation rules")
    } else {
        Vec::new()
    };

    let probes = std::fs::read_to_string(probes_path).expect("read probes");
    let mut out = String::new();
    for raw in probes.lines() {
        if raw.is_empty() {
            continue;
        }
        let text = raw.replace("\\n", "\n").replace("\\t", "\t");
        out.push_str("===|");
        out.push_str(raw);
        out.push('\n');
        let sentences = match &segmenter {
            Some(seg) => seg.segment(&text),
            None => vec![text.clone()],
        };
        for s in sentences {
            let mut sent = analyzer.raw(&s);
            if disambiguate {
                analysis::disambig::apply_all(&rules, &mut sent);
            }
            out.push_str("SENT|");
            out.push_str(&visible(&s));
            out.push('\n');
            for tok in &sent {
                out.push_str("TOK|");
                out.push_str(&visible(&tok.token));
                out.push('|');
                for (i, r) in tok.readings.iter().enumerate() {
                    if i > 0 {
                        out.push('#');
                    }
                    let lemma = r.lemma.as_deref().map(visible).unwrap_or_else(|| "∅".into());
                    let pos = r.pos.as_deref().unwrap_or("∅");
                    out.push_str(&lemma);
                    out.push('/');
                    out.push_str(pos);
                }
                out.push('\n');
            }
        }
    }
    print!("{out}");
    ExitCode::SUCCESS
}

fn visible(s: &str) -> String {
    s.replace('\n', "\\n").replace('\t', "\\t")
}
