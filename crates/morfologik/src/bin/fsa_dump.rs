//! Dump every byte sequence accepted by a CFSA2 `.dict`, one hex-encoded line
//! each, for byte-exact diffing against Morfologik's Java `fsa_dump`.
//!
//! Usage: fsa-dump <file.dict> [--text]
//!   --text  print sequences as UTF-8 (lossy) instead of hex.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: fsa-dump <file.dict> [--text]");
        return ExitCode::FAILURE;
    }
    let path = &args[1];
    let as_text = args.iter().any(|a| a == "--text");

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let fsa = match morfologik::Cfsa2::parse(&bytes) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("parse {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut count: u64 = 0;
    let mut out = String::new();
    fsa.for_each_sequence(|seq| {
        count += 1;
        if as_text {
            out.push_str(&String::from_utf8_lossy(seq));
        } else {
            for b in seq {
                out.push_str(&format!("{b:02x}"));
            }
        }
        out.push('\n');
        if out.len() > 1 << 20 {
            print!("{out}");
            out.clear();
        }
    });
    print!("{out}");
    eprintln!("{count} sequences");
    ExitCode::SUCCESS
}
