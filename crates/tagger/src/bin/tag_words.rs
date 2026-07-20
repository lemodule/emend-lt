//! Tag each word (one per line) in the same visible format as the Java
//! TagOracle: `WORD\t(tagged|untagged)\tlemma/POS|...` with readings sorted.
//!
//! Usage: tag-words <english.dict> <words.txt>

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: tag-words <english.dict> <words.txt>");
        return ExitCode::FAILURE;
    }
    let dict_path = &args[1];
    let info_path = dict_path
        .strip_suffix(".dict")
        .map(|s| format!("{s}.info"))
        .unwrap_or_default();
    let dict_bytes = match std::fs::read(dict_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {dict_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let info = std::fs::read_to_string(&info_path).unwrap_or_default();
    // Manual overlays live next to the resource dir, not the dict; look for a
    // sibling en/ resource path via env, else alongside the dict.
    let base = dict_path.strip_suffix("english.dict").unwrap_or("");
    let added = std::fs::read_to_string(format!("{base}added.txt")).unwrap_or_default();
    let removed = std::fs::read_to_string(format!("{base}removed.txt")).unwrap_or_default();
    let tagger = match tagger::EnglishTagger::load_with_manual(&dict_bytes, &info, &added, &removed) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("load tagger: {e}");
            return ExitCode::FAILURE;
        }
    };
    let words = match std::fs::read_to_string(&args[2]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", args[2]);
            return ExitCode::FAILURE;
        }
    };
    let mut out = String::new();
    for w in words.lines() {
        if w.is_empty() {
            continue;
        }
        let readings = tagger.tag_word(w);
        let tagged = if readings.is_empty() { "untagged" } else { "tagged" };
        let mut parts: Vec<String> = readings
            .iter()
            .map(|(lemma, pos)| {
                let l = if lemma.is_empty() { "_" } else { lemma.as_str() };
                format!("{l}/{pos}")
            })
            .collect();
        parts.sort();
        // Match the oracle: an untagged word shows its single null reading.
        if parts.is_empty() {
            parts.push("_/_".to_string());
        }
        out.push_str(&format!("{w}\t{tagged}\t{}\n", parts.join("|")));
    }
    print!("{out}");
    ExitCode::SUCCESS
}
