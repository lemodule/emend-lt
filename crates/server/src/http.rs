//! axum routing + the LanguageTool `/v2` JSON contract.
//!
//! Two endpoints the Scriptorium renderer actually calls:
//!   * `GET  /v2/languages` — the supported-language list.
//!   * `POST /v2/check`     — form (`text`, `language`, `level`, `disabledRules`,
//!                            `disabledCategories`) → `{ software, language, matches[] }`.
//!
//! Only `matches[].{offset,length,message,replacements[].value,rule.id,
//! rule.category.id}` are load-bearing for the app; the rest of each object is
//! populated for wire-compatibility with real LT clients.

use crate::engine::Engine;
use axum::{
    extract::{Form, State},
    http::{header, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// Shared server state: the loaded engine plus the configured CORS origin.
pub struct AppState {
    pub engine: Engine,
    /// The value echoed as `Access-Control-Allow-Origin` (LT `--allow-origin`).
    /// `None` sends no CORS header.
    pub allow_origin: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v2/languages", get(languages))
        .route("/v2/check", post(check).options(preflight))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// /v2/check
// ---------------------------------------------------------------------------

/// The form body LT accepts (we read the subset the app sends).
#[derive(Deserialize)]
struct CheckForm {
    text: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    level: Option<String>,
    #[serde(default, rename = "disabledRules")]
    disabled_rules: Option<String>,
    #[serde(default, rename = "disabledCategories")]
    disabled_categories: Option<String>,
}

async fn check(
    State(state): State<Arc<AppState>>,
    Form(form): Form<CheckForm>,
) -> Response {
    let text = match form.text {
        Some(t) => t,
        None => {
            return cors(
                &state,
                (StatusCode::BAD_REQUEST, "missing 'text' parameter").into_response(),
            )
        }
    };

    // `level=picky` would additionally enable `default="off"`/`tags="picky"`
    // rules. The engine currently compiles those out as unsupported (see NEXT.md
    // §6), so at any level we serve the `default` rule set; the app always sends
    // `level=default`, so this is a no-op today. Read to document the intent.
    let _picky = form.level.as_deref() == Some("picky");

    let disabled_rules: HashSet<String> = split_csv(&form.disabled_rules);
    let disabled_categories: HashSet<String> = split_csv(&form.disabled_categories);

    let matches = state.engine.check(&text, |rule_id, cat_id| {
        !disabled_rules.contains(rule_id) && !disabled_categories.contains(cat_id)
    });

    let language = form.language.unwrap_or_else(|| "en-US".to_string());
    let response = CheckResponse::build(&text, &language, &matches);
    cors(&state, Json(response).into_response())
}

async fn preflight(State(state): State<Arc<AppState>>) -> Response {
    cors(&state, StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// /v2/languages
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct Language {
    name: String,
    code: String,
    #[serde(rename = "longCode")]
    long_code: String,
}

async fn languages(State(state): State<Arc<AppState>>) -> Response {
    // English-first: this engine serves en-US only. `en` is advertised as an
    // alias so a client sending `language=en` resolves.
    let langs = vec![
        Language {
            name: "English".to_string(),
            code: "en".to_string(),
            long_code: "en".to_string(),
        },
        Language {
            name: "English (US)".to_string(),
            code: "en".to_string(),
            long_code: "en-US".to_string(),
        },
    ];
    cors(&state, Json(langs).into_response())
}

// ---------------------------------------------------------------------------
// Response shaping (LT JSON schema)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct CheckResponse {
    software: Software,
    language: LanguageInfo,
    matches: Vec<MatchJson>,
}

#[derive(Serialize)]
struct Software {
    name: &'static str,
    version: &'static str,
    #[serde(rename = "apiVersion")]
    api_version: u32,
    premium: bool,
    status: &'static str,
}

#[derive(Serialize)]
struct LanguageInfo {
    name: String,
    code: String,
    #[serde(rename = "detectedLanguage")]
    detected_language: DetectedLanguage,
}

#[derive(Serialize)]
struct DetectedLanguage {
    name: String,
    code: String,
}

#[derive(Serialize)]
struct MatchJson {
    message: String,
    #[serde(rename = "shortMessage")]
    short_message: String,
    replacements: Vec<Replacement>,
    offset: usize,
    length: usize,
    context: Context,
    sentence: String,
    rule: RuleJson,
}

#[derive(Serialize)]
struct Replacement {
    value: String,
}

#[derive(Serialize)]
struct Context {
    text: String,
    offset: usize,
    length: usize,
}

#[derive(Serialize)]
struct RuleJson {
    id: String,
    description: String,
    #[serde(rename = "issueType")]
    issue_type: String,
    category: CategoryJson,
}

#[derive(Serialize)]
struct CategoryJson {
    id: String,
    name: String,
}

impl CheckResponse {
    fn build(text: &str, language: &str, matches: &[analysis::GrammarMatch]) -> CheckResponse {
        let chars: Vec<char> = text.chars().collect();
        let matches = matches
            .iter()
            .map(|m| {
                let (context, ctx_off) = build_context(&chars, m.offset, m.length);
                MatchJson {
                    message: m.message.clone(),
                    short_message: String::new(),
                    replacements: m
                        .replacements
                        .iter()
                        .map(|v| Replacement { value: v.clone() })
                        .collect(),
                    offset: m.offset,
                    length: m.length,
                    context: Context {
                        text: context,
                        offset: ctx_off,
                        length: m.length,
                    },
                    sentence: String::new(),
                    rule: RuleJson {
                        id: m.rule_id.clone(),
                        description: String::new(),
                        issue_type: issue_type_for(&m.category_id),
                        category: CategoryJson {
                            id: m.category_id.clone(),
                            name: m.category_id.clone(),
                        },
                    },
                }
            })
            .collect();
        CheckResponse {
            software: Software {
                name: "emend-lt",
                version: env!("CARGO_PKG_VERSION"),
                api_version: 1,
                premium: false,
                status: "",
            },
            language: LanguageInfo {
                name: "English (US)".to_string(),
                code: language.to_string(),
                detected_language: DetectedLanguage {
                    name: "English (US)".to_string(),
                    code: language.to_string(),
                },
            },
            matches,
        }
    }
}

/// LT's context: a window of up to 40 chars around the match, ellipsised when
/// clipped. Returns the context string and the match's char offset within it.
fn build_context(chars: &[char], offset: usize, length: usize) -> (String, usize) {
    const WINDOW: usize = 40;
    let start = offset.saturating_sub(WINDOW / 2);
    let end = (offset + length + WINDOW / 2).min(chars.len());
    let mut ctx = String::new();
    let mut ctx_off = offset - start;
    if start > 0 {
        ctx.push_str("...");
        ctx_off += 3;
    }
    ctx.extend(&chars[start..end]);
    if end < chars.len() {
        ctx.push_str("...");
    }
    (ctx, ctx_off)
}

/// Coarse LT `issueType` mapping from the category id.
fn issue_type_for(category_id: &str) -> String {
    match category_id {
        "TYPOS" => "misspelling",
        "GRAMMAR" => "grammar",
        "TYPOGRAPHY" => "typographical",
        "PUNCTUATION" => "typographical",
        _ => "uncategorized",
    }
    .to_string()
}

fn split_csv(s: &Option<String>) -> HashSet<String> {
    s.as_deref()
        .map(|v| {
            v.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Attach the configured `Access-Control-Allow-Origin` (if any) to a response.
fn cors(state: &AppState, mut resp: Response) -> Response {
    if let Some(origin) = &state.allow_origin {
        if let Ok(val) = HeaderValue::from_str(origin) {
            resp.headers_mut()
                .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, val);
        }
        resp.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        );
        resp.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Content-Type"),
        );
    }
    let _ = Method::OPTIONS;
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_csv_trims_and_drops_empties() {
        let got = split_csv(&Some(" A, B ,,C ".to_string()));
        assert!(got.contains("A") && got.contains("B") && got.contains("C"));
        assert_eq!(got.len(), 3);
        assert!(split_csv(&None).is_empty());
    }

    #[test]
    fn context_windows_and_ellipsises() {
        // Match entirely inside a long text: both sides clipped -> both ellipses.
        let chars: Vec<char> = "x".repeat(100).chars().collect();
        let (ctx, off) = build_context(&chars, 50, 3);
        assert!(ctx.starts_with("...") && ctx.ends_with("..."));
        // The marked span sits at `off` within the returned context.
        assert_eq!(&ctx[off..off + 3], "xxx");
    }

    #[test]
    fn context_at_start_has_no_leading_ellipsis() {
        let chars: Vec<char> = "hello world".chars().collect();
        let (ctx, off) = build_context(&chars, 0, 5);
        assert!(!ctx.starts_with("..."));
        assert_eq!(off, 0);
        assert_eq!(&ctx[..5], "hello");
    }

    #[test]
    fn issue_type_maps_known_categories() {
        assert_eq!(issue_type_for("TYPOS"), "misspelling");
        assert_eq!(issue_type_for("GRAMMAR"), "grammar");
        assert_eq!(issue_type_for("SOMETHING_ELSE"), "uncategorized");
    }
}
