//! Coverage probe: extract every `<pattern>...</pattern>` block from a LT rule
//! file and try to parse each with the shared matcher, tallying how many parse
//! and which unsupported features (chunk, and/or) appear. This validates the
//! parser against real data, not just synthetic tests.
//!
//! Usage: pattern-coverage <grammar.xml|disambiguation.xml>

use std::collections::BTreeMap;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: pattern-coverage <rules.xml>");
        return ExitCode::FAILURE;
    }
    let raw = match std::fs::read_to_string(&args[1]) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", args[1]);
            return ExitCode::FAILURE;
        }
    };
    // Expand the file's <!ENTITY> macros before extracting patterns.
    let defs = matcher::parse_entity_defs(&raw);
    let xml = matcher::expand(&raw, &defs);
    eprintln!("expanded {} entity macros", defs.len());

    let mut total = 0;
    let mut parsed_ok = 0;
    let mut fully_supported = 0;
    let mut parse_err = 0;
    let mut tokens_total = 0usize;
    let mut feature_counts: BTreeMap<String, usize> = BTreeMap::new();

    for block in extract_blocks(&xml, "pattern") {
        total += 1;
        match matcher::parse_pattern(&block, false) {
            Ok(p) => {
                parsed_ok += 1;
                tokens_total += p.pattern.tokens.len();
                if p.unsupported.is_empty() {
                    fully_supported += 1;
                }
                for f in p.unsupported {
                    *feature_counts.entry(f).or_default() += 1;
                }
            }
            Err(_) => parse_err += 1,
        }
    }

    println!("file: {}", args[1]);
    println!("patterns:            {total}");
    println!("parsed without error:{parsed_ok}");
    println!("parse errors:        {parse_err}");
    println!(
        "fully supported:     {fully_supported} ({:.1}%)",
        100.0 * fully_supported as f64 / total.max(1) as f64
    );
    println!("avg tokens/pattern:  {:.1}", tokens_total as f64 / parsed_ok.max(1) as f64);
    println!("patterns using unsupported features:");
    for (f, c) in feature_counts {
        println!("  {f:10} {c}");
    }
    ExitCode::SUCCESS
}

/// Crude but sufficient: pull out each top-level `<tag ...>...</tag>` substring.
fn extract_blocks(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(s) = xml[pos..].find(&open) {
        let start = pos + s;
        // Skip self-closing <pattern/> (none in practice) and ensure it's a tag.
        if let Some(e) = xml[start..].find(&close) {
            let end = start + e + close.len();
            out.push(xml[start..end].to_string());
            pos = end;
        } else {
            break;
        }
    }
    out
}
