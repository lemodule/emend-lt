//! Report whether each whitespace-separated word on stdin is misspelled per the
//! `en_US` speller. Usage: spell-check <en_US.dict>
use std::io::Read;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let dictp = &a[1];
    let info = std::fs::read_to_string(dictp.strip_suffix(".dict").map(|s| format!("{s}.info")).unwrap()).unwrap_or_default();
    let db = std::fs::read(dictp).unwrap();
    let sp = morfologik::Speller::load(&db, &info).unwrap();
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).unwrap();
    for w in s.split_whitespace() {
        println!("{}\t{}", if sp.is_misspelled(w) { "MISSPELLED" } else { "ok" }, w);
    }
}
