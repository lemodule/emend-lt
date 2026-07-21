//! Morfologik CFSA2 dictionary reader for LanguageTool `.dict`/`.info` files.
//!
//! Phase 1.1 of the engine: the FSA primitive that backs both spelling lookups
//! (`MORFOLOGIK_RULE_*`) and POS tagging. English-first.

pub mod dict;
pub mod fsa;
pub mod synth;

pub use dict::{Dictionary, DictEntry, Encoder};
pub use fsa::{Cfsa2, FsaError};
pub use synth::Synthesizer;
