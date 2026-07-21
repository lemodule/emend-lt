use analysis::grammar::{check_sentence, parse_grammar_rules};
use analysis::{parse_disambig_rules, Analyzer};
use matcher::{expand, parse_entity_defs};
use morfologik::Synthesizer;
fn main(){
    let a:Vec<String>=std::env::args().collect();
    let (dictp,disp,gramp,srx,text)=(&a[1],&a[2],&a[3],&a[4],&a[5]);
    let base=dictp.strip_suffix("english.dict").unwrap_or("");
    let info=std::fs::read_to_string(dictp.strip_suffix(".dict").map(|s|format!("{s}.info")).unwrap()).unwrap_or_default();
    let db=std::fs::read(dictp).unwrap();
    let added=std::fs::read_to_string(format!("{base}added.txt")).unwrap_or_default();
    let removed=std::fs::read_to_string(format!("{base}removed.txt")).unwrap_or_default();
    let tagger=tagger::EnglishTagger::load_with_manual(&db,&info,&added,&removed).unwrap();
    let an=Analyzer::new(&tagger);
    let dx=std::fs::read_to_string(disp).unwrap();let dd=parse_entity_defs(&dx);
    let dr=parse_disambig_rules(&expand(&dx,&dd)).unwrap();
    let gx=std::fs::read_to_string(gramp).unwrap();let gd=parse_entity_defs(&gx);
    let gr=parse_grammar_rules(&expand(&gx,&gd)).unwrap();
    let sx=std::fs::read_to_string(srx).unwrap();
    let seg=segmenter::Segmenter::from_srx(&sx,"en-US").unwrap();
    // Optional synthesizer: args[6]=english_synth.dict, args[7]=english_tags.txt.
    let synth = if a.len()>=8 {
        let sdict=std::fs::read(&a[6]).unwrap();
        let sinfo=std::fs::read_to_string(a[6].strip_suffix(".dict").map(|s|format!("{s}.info")).unwrap_or_default()).unwrap_or_default();
        let stags=std::fs::read_to_string(&a[7]).unwrap();
        Some(Synthesizer::load(&sdict,&sinfo,&stags).unwrap())
    } else { None };
    let synth_ref=synth.as_ref();
    // char offset of each sentence within full text
    let mut char_off=0usize; let mut byte_off=0usize;
    for s in seg.segment(text){
        let mut t=an.raw(&s); analysis::disambig::apply_all(&dr,&mut t);
        for m in check_sentence(&gr,&t,&s,char_off,synth_ref){
            println!("{}\t{}\t{}\t{}", m.rule_id, m.offset, m.length, m.replacements.join("|"));
        }
        char_off += s.chars().count();
        byte_off += s.len();
        let _=byte_off;
    }
}
