//! Run only the spelling rule (MORFOLOGIK_RULE_EN_US) over stdin text and print
//! matches as `offset<TAB>length<TAB>word<TAB>suggestions`, for diffing against
//! the live `/v2/check`.
//!
//! Usage: spell-text <english.dict> <disambiguation.xml> <segment.srx>
//!                   <en_US.dict> <ignore+spelling...>,<prohibit.txt>
use std::io::Read;
use analysis::{parse_disambig_rules, Analyzer, SpellRule};
use matcher::{expand, parse_entity_defs};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (dictp, disp, srx, spdict) = (&a[1], &a[2], &a[3], &a[4]);
    // a[5..N-2] = ignore/spelling files; a[N-2] = multiwords.txt; a[N-1] = prohibit.txt
    let ignore_paths = &a[5..a.len() - 2];
    let multiwords_path = &a[a.len() - 2];
    let prohibit_path = &a[a.len() - 1];

    let base = dictp.strip_suffix("english.dict").unwrap_or("");
    let info = std::fs::read_to_string(dictp.strip_suffix(".dict").map(|s| format!("{s}.info")).unwrap()).unwrap_or_default();
    let db = std::fs::read(dictp).unwrap();
    let added = std::fs::read_to_string(format!("{base}added.txt")).unwrap_or_default();
    let removed = std::fs::read_to_string(format!("{base}removed.txt")).unwrap_or_default();
    let tagger = tagger::EnglishTagger::load_with_manual(&db, &info, &added, &removed).unwrap();
    let an = Analyzer::new(&tagger);
    let dx = std::fs::read_to_string(disp).unwrap();
    let dd = parse_entity_defs(&dx);
    let dr = parse_disambig_rules(&expand(&dx, &dd)).unwrap();
    let sx = std::fs::read_to_string(srx).unwrap();
    let seg = segmenter::Segmenter::from_srx(&sx, "en-US").unwrap();

    let spinfo = std::fs::read_to_string(spdict.strip_suffix(".dict").map(|s| format!("{s}.info")).unwrap()).unwrap_or_default();
    let spdb = std::fs::read(spdict).unwrap();
    let ignore_texts: Vec<String> = ignore_paths.iter().map(|p| std::fs::read_to_string(p).unwrap_or_default()).collect();
    let ignore_refs: Vec<&str> = ignore_texts.iter().map(|s| s.as_str()).collect();
    let multiwords = std::fs::read_to_string(multiwords_path).unwrap_or_default();
    let prohibit = std::fs::read_to_string(prohibit_path).unwrap_or_default();
    let rule = SpellRule::load(&spdb, &spinfo, &ignore_refs, &multiwords, &prohibit).unwrap();

    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).unwrap();
    let mut char_off = 0usize;
    for s in seg.segment(&text) {
        let mut t = an.raw(&s);
        analysis::disambig::apply_all(&dr, &mut t);
        for m in rule.check_sentence(&t, &s, char_off) {
            println!("{}\t{}\t{}", m.offset, m.length, m.replacements.join("|"));
        }
        char_off += s.chars().count();
    }
}
