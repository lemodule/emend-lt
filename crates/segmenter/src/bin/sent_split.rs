//! Split each line of a probe file into sentences using an SRX rule set, in the
//! same visible format as the Java oracle (`SENT<len>|<text>` per sentence),
//! for direct diffing.
//!
//! Usage: sent-split <segment.srx> <lang-code> <probes.txt>

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: sent-split <segment.srx> <lang-code> <probes.txt>");
        return ExitCode::FAILURE;
    }
    let xml = match std::fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", args[1]);
            return ExitCode::FAILURE;
        }
    };
    let seg = match segmenter::Segmenter::from_srx(&xml, &args[2]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("build segmenter: {e}");
            return ExitCode::FAILURE;
        }
    };
    let probes = match std::fs::read_to_string(&args[3]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", args[3]);
            return ExitCode::FAILURE;
        }
    };

    let mut idx = 0;
    for raw in probes.lines() {
        if raw.is_empty() {
            continue;
        }
        let text = raw.replace("\\n", "\n").replace("\\t", "\t");
        println!("===PROBE {idx}===");
        idx += 1;
        for s in seg.segment(&text) {
            let vis = s.replace('\n', "\\n").replace('\t', "\\t");
            println!("SENT{}|{}", s.chars().count(), vis);
        }
    }
    ExitCode::SUCCESS
}
