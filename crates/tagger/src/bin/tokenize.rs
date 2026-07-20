//! End-to-end word tokenization: EnglishWordTokenizer backed by the real
//! EnglishTagger, emitting the same `WORDS|tok␟tok␟...` format as the Java
//! oracle for direct diffing.
//!
//! Usage: tokenize <english.dict> <probes.txt>

use std::process::ExitCode;
use tokenizer::EnglishWordTokenizer;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tokenize <english.dict> <probes.txt>");
        return ExitCode::FAILURE;
    }
    let dict_path = &args[1];
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
    let tok = EnglishWordTokenizer::new(&tagger);

    let probes = std::fs::read_to_string(&args[2]).expect("read probes");
    let mut out = String::new();
    for raw in probes.lines() {
        if raw.is_empty() {
            continue;
        }
        let text = raw.replace("\\n", "\n").replace("\\t", "\t");
        out.push_str("WORDS|");
        for t in tok.tokenize(&text) {
            let vis = t.replace('\n', "\\n").replace('\t', "\\t");
            out.push_str(&vis);
            out.push('\u{241F}');
        }
        out.push('\n');
    }
    print!("{out}");
    ExitCode::SUCCESS
}
