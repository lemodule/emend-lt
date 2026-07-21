//! Self-oracle for the grammar engine: run every supported rule against its own
//! `<example correction="...">` cases (LT's built-in test corpus) and report how
//! many fire at the marked span with the expected replacements.
//!
//! Usage: grammar-examples <english.dict> <disambiguation.xml> <grammar.xml>

use std::process::ExitCode;

use analysis::grammar::{check_one, parse_grammar_rules};
use analysis::{parse_disambig_rules, Analyzer};
use matcher::{expand, parse_entity_defs};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: grammar-examples <english.dict> <disambiguation.xml> <grammar.xml>");
        return ExitCode::FAILURE;
    }
    let dict_path = &args[1];
    let base = dict_path.strip_suffix("english.dict").unwrap_or("");
    let info_path = dict_path.strip_suffix(".dict").map(|s| format!("{s}.info")).unwrap_or_default();
    let dict_bytes = std::fs::read(dict_path).expect("read dict");
    let info = std::fs::read_to_string(&info_path).unwrap_or_default();
    let added = std::fs::read_to_string(format!("{base}added.txt")).unwrap_or_default();
    let removed = std::fs::read_to_string(format!("{base}removed.txt")).unwrap_or_default();
    let tagger =
        tagger::EnglishTagger::load_with_manual(&dict_bytes, &info, &added, &removed).unwrap();
    let analyzer = Analyzer::new(&tagger);

    let dis_xml = std::fs::read_to_string(&args[2]).expect("read disambiguation.xml");
    let dis_defs = parse_entity_defs(&dis_xml);
    let dis_rules = parse_disambig_rules(&expand(&dis_xml, &dis_defs)).unwrap();

    let gram_xml = std::fs::read_to_string(&args[3]).expect("read grammar.xml");
    let gram_defs = parse_entity_defs(&gram_xml);
    let gram_rules = parse_grammar_rules(&expand(&gram_xml, &gram_defs)).unwrap();

    let total_rules = gram_rules.len();
    let supported: Vec<_> = gram_rules.iter().filter(|r| r.unsupported.is_empty()).collect();

    let (mut pos_ok, mut pos_bad, mut neg_ok, mut neg_bad) = (0, 0, 0, 0);
    let mut fail_samples: Vec<String> = Vec::new();

    for rule in &supported {
        for ex in &rule.examples {
            // One sentence per example.
            let mut toks = analyzer.raw(&ex.text);
            analysis::disambig::apply_all(&dis_rules, &mut toks);
            let matches = check_one(rule, &toks, &ex.text, 0);

            match (&ex.correction, ex.marker) {
                (Some(corr), Some((moff, mlen))) => {
                    // Positive: some match hits the marker with expected replacements.
                    let hit = matches.iter().find(|m| m.offset == moff && m.length == mlen);
                    let ok = hit.map(|m| replacements_match(&m.replacements, corr)).unwrap_or(false);
                    if ok {
                        pos_ok += 1;
                    } else {
                        pos_bad += 1;
                        if fail_samples.len() < 25 {
                            let got = hit.map(|m| m.replacements.join("|")).unwrap_or_else(|| "<no match at marker>".into());
                            fail_samples.push(format!(
                                "POS {} : want [{}] got [{}]  «{}»",
                                rule.id, corr.join("|"), got, ex.text
                            ));
                        }
                    }
                }
                // `type="triggers_error"` examples are expected to fire but
                // carry no correction — they are neither positive nor negative.
                (None, _) if ex.triggers_error => {}
                (None, _) => {
                    // Negative: the rule must not fire (anywhere).
                    if matches.is_empty() {
                        neg_ok += 1;
                    } else {
                        neg_bad += 1;
                        if fail_samples.len() < 25 {
                            fail_samples.push(format!(
                                "NEG {} : fired [{}]  «{}»",
                                rule.id, matches[0].replacements.join("|"), ex.text
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    println!("grammar rules: {total_rules}  (supported: {})", supported.len());
    println!(
        "positive examples: {}/{} pass ({:.1}%)",
        pos_ok,
        pos_ok + pos_bad,
        pct(pos_ok, pos_ok + pos_bad)
    );
    println!(
        "negative examples: {}/{} pass ({:.1}%)",
        neg_ok,
        neg_ok + neg_bad,
        pct(neg_ok, neg_ok + neg_bad)
    );
    println!("--- sample failures ---");
    for s in &fail_samples {
        println!("  {s}");
    }
    ExitCode::SUCCESS
}

/// Replacements match the `correction` alternatives (order-independent).
fn replacements_match(got: &[String], want: &[String]) -> bool {
    let g: std::collections::BTreeSet<_> = got.iter().collect();
    let w: std::collections::BTreeSet<_> = want.iter().collect();
    g == w
}

fn pct(a: usize, b: usize) -> f64 {
    if b == 0 { 0.0 } else { 100.0 * a as f64 / b as f64 }
}
