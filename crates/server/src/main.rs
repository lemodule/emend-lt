//! `emend-server` — the Phase 2 HTTP server.
//!
//! Loads the LanguageTool data files once and serves the `/v2/check` +
//! `/v2/languages` contract the Scriptorium app calls (default port 8010).
//!
//! Usage:
//!   emend-server \
//!     --dict <english.dict> --disambiguation <disambiguation.xml> \
//!     --grammar <grammar.xml> --srx <segment.srx> \
//!     [--synth-dict <english_synth.dict> --synth-tags <english_tags.txt>] \
//!     [--spell-dict <en_US.dict> --spell-ignore <ignore.txt> ...] \
//!     [--multiwords <multiwords.txt>] [--prohibit <prohibit.txt>] \
//!     [--port 8010] [--allow-origin "*"]
//!
//! A `--data-dir <DIR>` fills any unset path from the conventional filenames
//! under DIR, so a fully-extracted directory needs only `--data-dir`.

mod engine;
mod http;

use engine::{Engine, EngineConfig};
use http::{router, AppState};
use std::path::Path;
use std::sync::Arc;

struct Args {
    port: u16,
    allow_origin: Option<String>,
    cfg: EngineConfig,
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\nrun with --help for usage");
            std::process::exit(2);
        }
    };

    eprintln!("emend-server: loading data files…");
    let engine = match Engine::load(&args.cfg) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("failed to load engine: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("emend-server: engine ready.");

    let state = Arc::new(AppState {
        engine,
        allow_origin: args.allow_origin,
    });

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let app = router(state);
        let addr = format!("0.0.0.0:{}", args.port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| {
                eprintln!("failed to bind {addr}: {e}");
                std::process::exit(1);
            });
        eprintln!("emend-server: listening on http://{addr}  (POST /v2/check, GET /v2/languages)");
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .expect("server error");
    });
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("\nemend-server: shutting down.");
}

fn parse_args() -> Result<Args, String> {
    let mut port = 8010u16;
    let mut allow_origin: Option<String> = None;
    let mut data_dir: Option<String> = None;
    let mut dict: Option<String> = None;
    let mut disambiguation: Option<String> = None;
    let mut grammar: Option<String> = None;
    let mut srx: Option<String> = None;
    let mut synth_dict: Option<String> = None;
    let mut synth_tags: Option<String> = None;
    let mut spell_dict: Option<String> = None;
    let mut spell_ignore: Vec<String> = Vec::new();
    let mut multiwords: Option<String> = None;
    let mut prohibit: Option<String> = None;

    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        let mut take = || {
            i += 1;
            argv.get(i)
                .cloned()
                .ok_or_else(|| format!("{a} needs a value"))
        };
        match a.as_str() {
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            "--port" => port = take()?.parse().map_err(|_| "invalid --port")?,
            "--allow-origin" => allow_origin = Some(take()?),
            "--data-dir" => data_dir = Some(take()?),
            "--dict" => dict = Some(take()?),
            "--disambiguation" => disambiguation = Some(take()?),
            "--grammar" => grammar = Some(take()?),
            "--srx" => srx = Some(take()?),
            "--synth-dict" => synth_dict = Some(take()?),
            "--synth-tags" => synth_tags = Some(take()?),
            "--spell-dict" => spell_dict = Some(take()?),
            "--spell-ignore" => spell_ignore.push(take()?),
            "--multiwords" => multiwords = Some(take()?),
            "--prohibit" => prohibit = Some(take()?),
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }

    // Fill unset paths from the conventional filenames under --data-dir.
    if let Some(dir) = &data_dir {
        let d = Path::new(dir);
        let fill = |slot: &mut Option<String>, name: &str| {
            if slot.is_none() {
                let p = d.join(name);
                if p.exists() {
                    *slot = Some(p.to_string_lossy().into_owned());
                }
            }
        };
        fill(&mut dict, "english.dict");
        fill(&mut disambiguation, "disambiguation.xml");
        fill(&mut grammar, "grammar.xml");
        fill(&mut srx, "segment.srx");
        fill(&mut synth_dict, "english_synth.dict");
        fill(&mut synth_tags, "english_tags.txt");
        fill(&mut spell_dict, "en_US.dict");
        fill(&mut multiwords, "multiwords.txt");
        fill(&mut prohibit, "prohibit.txt");
        if spell_ignore.is_empty() {
            for name in ["ignore.txt", "spelling.txt", "spelling_en-US.txt"] {
                let p = d.join(name);
                if p.exists() {
                    spell_ignore.push(p.to_string_lossy().into_owned());
                }
            }
        }
    }

    let cfg = EngineConfig {
        dict: dict.ok_or("--dict (or --data-dir) is required")?,
        disambiguation_xml: disambiguation.ok_or("--disambiguation (or --data-dir) is required")?,
        grammar_xml: grammar.ok_or("--grammar (or --data-dir) is required")?,
        srx: srx.ok_or("--srx (or --data-dir) is required")?,
        synth_dict,
        synth_tags,
        spell_dict,
        spell_ignore,
        spell_multiwords: multiwords,
        spell_prohibit: prohibit,
    };

    Ok(Args {
        port,
        allow_origin,
        cfg,
    })
}

fn print_usage() {
    eprintln!(
        "emend-server — LanguageTool-compatible /v2 HTTP server (English)\n\n\
         Required (or provide --data-dir with conventional filenames):\n\
         \x20 --dict <english.dict>            POS/tagger dictionary\n\
         \x20 --disambiguation <file.xml>      disambiguation.xml\n\
         \x20 --grammar <file.xml>             grammar.xml\n\
         \x20 --srx <segment.srx>              sentence-segmentation rules\n\n\
         Optional subsystems:\n\
         \x20 --synth-dict <english_synth.dict> --synth-tags <english_tags.txt>\n\
         \x20 --spell-dict <en_US.dict> [--spell-ignore <list.txt> ...]\n\
         \x20 --multiwords <multiwords.txt> --prohibit <prohibit.txt>\n\n\
         Server:\n\
         \x20 --port <N>            default 8010\n\
         \x20 --allow-origin <O>    echoed Access-Control-Allow-Origin (e.g. \"*\")\n\
         \x20 --data-dir <DIR>      fill any unset path from DIR/<conventional-name>\n"
    );
}
