//! Look up words in a Morfologik dictionary (`.dict` + `.info`).
//!
//! Usage:
//!   dict-lookup <file.dict> contains <word>...   # spelling membership
//!   dict-lookup <file.dict> lookup   <word>...   # POS stem/tag decode

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: dict-lookup <file.dict> <contains|lookup> <word>...");
        return ExitCode::FAILURE;
    }
    let dict_path = &args[1];
    let mode = &args[2];
    let words = &args[3..];

    let info_path = dict_path.strip_suffix(".dict").map(|s| format!("{s}.info"));
    let dict_bytes = match std::fs::read(dict_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {dict_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let info_text = info_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();

    let dict = match morfologik::Dictionary::load(&dict_bytes, &info_text) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("load {dict_path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match mode.as_str() {
        "contains" => {
            for w in words {
                println!("{}\t{}", w, dict.contains(w));
            }
        }
        "lookup" => {
            for w in words {
                let entries = dict.lookup(w);
                if entries.is_empty() {
                    println!("{w}\t<none>");
                } else {
                    for e in entries {
                        println!("{w}\t{}\t{}", e.stem, e.tag);
                    }
                }
            }
        }
        other => {
            eprintln!("unknown mode: {other}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
