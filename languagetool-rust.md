https://languagetool.org/download/LanguageTool-6.6.zip

# Roadmap: Replacing Java LanguageTool with a Rust reimplementation (LLM-assisted)

## Context

Scriptorium bundles **LanguageTool 6.6 + a stripped JDK 21 JRE** into the desktop app. This is
the single heaviest dependency (100MB+ of LT rule/resource data, plus tens of MB of JRE) and it's
the direct cause of the current macOS notarization pain — `sign-languagetool-natives.cjs` has to
pre-sign _hundreds_ of Mach-O files in the JRE and codesign embedded `.dylib`/`.jnilib` natives
inside every LT JAR.

Goal: replace the Java process with a native **Rust binary** that speaks the same local HTTP
contract, so the JVM, the JARs, and the native-signing dance all disappear. The renderer stays
untouched. This is an LLM-assisted rewrite of LanguageTool's _engine_, reusing LanguageTool's own
data files unchanged.

This document is the "big lines" feasibility roadmap the user asked for — the user is the judge of
whether to pursue it.

## Why this is tractable (the key insight)

LanguageTool is ~10% engine, ~90% data. The Rust rewrite does **not** translate LT's Java line by
line. It builds a Rust engine that **reads LanguageTool's existing data files**:

- XML pattern rules (`grammar.xml` per language)
- Morfologik FSA dictionaries (`.dict`/`.info`) for spelling + POS tagging
- SRX sentence segmentation + disambiguation XML
- The subset of hand-coded Java rules that your enabled languages actually load

## The replacement seam (verified)

The app talks to LanguageTool **only** over HTTP on `localhost:8010`, from the renderer via
`fetch`. Two endpoints:

1. `GET /v2/languages` — health/readiness probe. App inspects only `response.ok` +
   `access-control-allow-origin` header. Body not parsed.
2. `POST /v2/check` — `application/x-www-form-urlencoded` with fields `text`, `language`,
   `level`, `disabledRules` (CSV), `disabledCategories` (CSV). Response JSON — **the app reads only**:
   `matches[].{offset, length, message, replacements[].value (first 5), rule.id, rule.category.id}`.
   Everything else in the response (`software`, `context`, `sentence`, `shortMessage`, `issueType`,
   `subId`) is typed but unused.

So the Rust binary's entire external obligation is: those two endpoints, honoring
`disabledRules`/`disabledCategories`, echoing the `--allow-origin` CORS origin exactly.

Load-bearing detail: `isServedOriginCorrect()`
([languagetool.ts:298](packages/desktop/src/main/languagetool.ts#L298)) force-restarts a server
whose `access-control-allow-origin` doesn't match the expected origin. The Rust server **must**
echo the origin passed on its command line.

Languages in scope (must match, currently a slight mismatch to fix): en-US/GB/AU/CA, fr,
de-DE/AT/CH, es, pt-PT/BR, it, pl. (`nl-BE` appears in the UI list at
[projects.impl.ts:470](packages/frontend/src/dexie/infra/projects.impl.ts#L470) but is **not**
bundled — flag/resolve during the port.)

## Big-line phases

### Phase 0 — Scope down to what the app hits

Enumerate the rules actually fired for the enabled languages at `level=default` with the app's
`disabledRules`/`disabledCategories` applied. This defines the real target set (a fraction of LT's
full rule base) and the priority order for the port.

### Phase 1 — Build the Rust core (bottom-up dependency order)

1. **Morfologik FSA reader** — load `.dict`/`.info`, do finite-state lookups for spelling + POS
   tags. Single most important primitive; the `fst` crate gets you most of the way. LLM-friendly
   because the format is well-specified.
2. **Tokenizer + SRX sentence splitter** — reproduce LT's segmentation from its SRX rules.
3. **POS tagger + disambiguator** — driven by dictionaries + disambiguation XML.
4. **XML pattern-rule engine** — parse `grammar.xml`, execute token-pattern semantics
   (tokens, exceptions, antipatterns, `<suggestion>`, skip/min/max). Biggest chunk; ported
   rule-feature by rule-feature.
5. **Hand-coded rules** — LT's Java-class rules have no data file; reimplement each in Rust
   individually. Port only the ones your enabled languages load; skip the long tail initially.

### Phase 2 — Serve the identical HTTP contract

Wrap the core in a tiny Rust HTTP server (`axum`/`hyper`) exposing `/v2/check` + `/v2/languages`
with the exact request parsing and response JSON above, plus the CORS `--allow-origin` CLI flag.
Drop-in: the renderer changes nothing.

### Phase 3 — Differential testing = the correctness oracle

This is what makes an LLM port _trustworthy_. Run stock Java LT 6.6 and the Rust binary side by
side over a large multi-language corpus; diff the `matches[]` JSON per sentence; drive the LLM to
close each mismatch. Prioritize by rule hit-frequency to reach ~95% behavioral parity fast, then
chase the tail. This gives ground truth for every input — no guessing.

### Phase 4 — Swap the backend in the desktop app

Only two areas change; everything else in `languagetool.ts` (PID reaping, health monitor,
exponential-backoff restart, graceful shutdown) is transport-agnostic and stays:

- **Spawn + path resolution** in
  [languagetool.ts](packages/desktop/src/main/languagetool.ts): replace the `spawn(jreBinary, [..
"org.languagetool.server.HTTPServer" ..])` (~L403-428) and the JRE/JAR resolvers
  (`resolveJreBinaryPath`/`resolveJarPath`, ~L54-79) to launch the Rust binary. Keep port 8010,
  `--allow-origin`, and the two endpoints. Keep the `GrammarMode = "languagetool"` string literal
  ([grammar-settings.ts](packages/desktop/src/main/grammar-settings.ts)) to avoid a wide rename.
- **Bundling/signing**: replace [prepare-languagetool.sh](scripts/prepare-languagetool.sh)
  (JDK download + jlink + LT strip) with a per-platform Rust cross-compile that emits one signed
  binary + the data files. Gut [sign-languagetool-natives.cjs](packages/desktop/scripts/sign-languagetool-natives.cjs)
  down to signing the single Rust binary (delete the JRE Mach-O walk + per-JAR native extraction —
  keep only the unrelated onnxruntime TTS dedupe, or move that elsewhere). Update the per-OS
  `extraResources` in [package.json](packages/desktop/package.json) (L70-160).

## Honest size read (user's table is right)

The prize is real: removing the JRE (tens of MB) + JARs and yielding a single-digit-to-low-double-
digit-MB native binary. Two caveats so the judgment is fully informed: the **Morfologik
dictionaries and any n-gram data are unchanged in size** (you already had this right), and
compiling XML into the binary saves parse _time_, not much disk — XML is small next to the
dictionaries. The overwhelming win is deleting the JVM, which is exactly what's tangling the
notarization work on this branch.

## Effort honesty

This is a real project, not a weekend. Phase 1.4 (XML engine) and 1.5 (hand-coded rules) are the
long poles. The differential-testing harness (Phase 3) is what de-risks it and is worth building
first-class. Realistic staging: get English-only to parity end-to-end as a proof point before
adding fr/de/es/pt/it/pl.

## Verification (per stage)

- **Core**: unit tests per primitive (FSA lookups, tokenization) against known LT outputs.
- **Parity**: the Phase 3 differential harness — % of `matches[]` agreement vs Java LT 6.6 across a
  corpus, per language, tracked as the headline metric.
- **In-app**: point the desktop app's port 8010 at the Rust binary; run
  [e2e/spellcheck.spec.ts](packages/desktop/e2e/spellcheck.spec.ts) and the main-process
  `languagetool.test.ts` / `languagetool.property.test.ts`; confirm the renderer linter
  ([languagetool-linter.ts](packages/frontend/src/components/editor/plugins/languagetool/languagetool-linter.ts))
  gets identical `LintResult`s and CORS passes `isServedOriginCorrect()`.
- **Packaging**: a full `electron-builder` mac build that notarizes _without_ the JRE Mach-O
  signing loop.

# Project Roadmap: A Rust Grammar-Checking Engine Compatible with LanguageTool

> Standalone document — written to be read on its own once moved into its own repository.
> Not affiliated with, endorsed by, or a fork of the LanguageTool project. "LanguageTool" (LT) is
> referenced only descriptively, to explain the data formats and HTTP API this project is
> compatible with.

## What this is

A from-scratch **Rust** implementation of a grammar/spell-checking engine that:

- **Reads LanguageTool's own rule and dictionary files unchanged** (XML pattern rules, Morfologik
  FSA dictionaries, SRX segmentation, disambiguation XML) rather than re-encoding that data.
- **Serves the same local HTTP API** that LanguageTool's embedded server exposes
  (`GET /v2/languages`, `POST /v2/check`), so any application currently embedding LanguageTool's
  Java server can swap in this engine as a drop-in backend with no client-side changes.
- Has **no JVM dependency** — a single native binary per platform.

The motivating case is desktop apps that bundle LanguageTool's Java server as a local sidecar
process purely for its `/v2/check` HTTP contract: they pay a large JVM/JAR footprint, slow cold
start, and painful native-library code-signing/notarization, none of which is inherent to the
_grammar checking_ itself — it's a byproduct of the delivery mechanism (a bundled JRE).

## Why this is tractable

LanguageTool is architecturally **~10% engine, ~90% data**. The bulk of what makes it powerful —
thousands of pattern rules per language, morphological dictionaries — lives in XML and binary FSA
files, not in the Java code that interprets them. That means this project does not translate LT's
Java source line-by-line; it builds a new interpreter for the same data formats:

- XML pattern rules (`grammar.xml` per language) — token/pattern matching, exceptions,
  antipatterns, suggestions, unification, filters.
- Morfologik finite-state-automaton dictionaries (`.dict`/`.info`) — spelling lookups and
  morphological/POS tagging.
- SRX-based sentence segmentation and disambiguation XML — tokenization and POS disambiguation.
- A bounded set of **hand-coded rules** (the subset of LT's Java rule _classes_ that have no XML
  data file — these must be reimplemented individually per rule, in Rust).

## The HTTP contract to match

The entire external surface a compatible engine must implement is small:

1. **`GET /v2/languages`** — a readiness probe. Must return `200 OK` with the configured
   CORS origin echoed back on `access-control-allow-origin`. Response body content is not required
   to be consumed by typical clients, but returning a LT-shaped language list is good practice for
   compatibility with other LT clients.
2. **`POST /v2/check`** — the actual check request, `application/x-www-form-urlencoded`, with at
   minimum these fields honored: `text`, `language`, `level`, `disabledRules` (CSV of rule IDs),
   `disabledCategories` (CSV of category IDs). Response: JSON with a `matches[]` array; each match
   should include at least `offset`, `length`, `message`, `replacements[].value`, `rule.id`,
   `rule.category.id` — the fields real-world LT clients typically rely on. Extra LT response
   fields (`software`, `context`, `sentence`, `shortMessage`, `issueType`, `subId`) can be included
   for closer compatibility but are optional for most clients.

A load-bearing detail for any embedding app: the server must be started with a **configurable CORS
origin** (a `--allow-origin <origin>` style flag) and must echo that exact origin on every
response — many embedding apps validate this and treat a mismatch as a server that needs
restarting.

Target language set is a project decision, not a technical constraint — the engine should support
adding a language by dropping in that language's LT-format rule/dictionary files, without code
changes (data-driven, per the architecture above).

## Licensing

**License this project LGPL-2.1**, matching LanguageTool's own license. This isn't a style
preference — it follows from what the project actually does:

- LLM-assisted translation of LanguageTool's Java source produces a work that should be treated as
  a **derivative of LGPL-2.1 code** — the copyright status of LLM-generated translations is legally
  unsettled, so the safe and honest assumption is that LGPL obligations carry over. Do not license
  this project MIT/Apache/etc.
- The project also redistributes LanguageTool's **rule files and dictionaries** as data. Most are
  LGPL, but **verify each shipped dictionary's individual license** before distribution — they are
  not all identical.
- LGPL is designed for exactly this shape of dependency: as long as the engine is consumed as a
  **separate process over a network/IPC boundary** (the recommended architecture — see below),
  applications embedding it may remain proprietary. Keep that separation intact; don't statically
  link the engine into a proprietary binary in a way that blurs the LGPL boundary.
- Get LGPL compliance (attribution, source availability, per-dictionary licenses) confirmed by
  someone qualified before the first public release. This document is not legal advice.

## Naming

Avoid using "LanguageTool" in the project name — it's very likely a protected trademark, and a
literal clone-sounding name invites a takedown request regardless of the code's legality. Referring
to LanguageTool _descriptively_ in documentation ("compatible with LanguageTool's rule format and
`/v2/check` API") is fine; using it as or within the product name is not.

A name built around **"lt"** as an abbreviation (rather than spelling out the full trademarked
term) is a reasonable, low-risk direction — e.g. combining "lt" with a Rust-community marker
(Ferris/oxide/rs). Whatever is chosen, verify availability on crates.io and GitHub before
finalizing, and keep a clear "not affiliated with LanguageTool" disclaimer in the README.

## Architecture recommendation: separate repository, sidecar process

- **Own repository**, independent release cadence from any consuming application. The engine's
  natural consumption pattern is as a **versioned binary artifact** (cross-compiled per platform,
  published as a release), fetched by whatever build system a consuming app already uses to fetch
  its current LanguageTool JAR/JRE bundle — the same artifact-boundary pattern, just a different
  binary.
- **Runs as a sidecar process communicating over local HTTP**, not as an in-process native
  library/addon. This is both the simplest integration path for any app that already spawns
  LanguageTool's Java server as a child process (near-zero change on the client side — same port,
  same two endpoints) and the cleanest LGPL boundary (see Licensing). An in-process native-module
  embedding (e.g. via a Rust↔host-language FFI/addon bridge) is a legitimate later optimization but
  should not be the starting design — it couples the engine's build to the host app's build and
  discards the clean process boundary.
- A native binary needs to be code-signed the same way any other bundled native executable would
  be in a consuming app's packaging pipeline (e.g. macOS hardened runtime + notarization) — but
  this is _one_ binary to sign, versus signing a JVM's worth of Mach-O files plus every native
  library embedded in third-party JARs.

## Implementation phases (bottom-up)

**Phase 0 — Scope.** Enumerate the actual rule set to target: which rules fire, for which
languages, under the specific `level` and `disabledRules`/`disabledCategories` a typical client
uses. This is a large filter — real-world usage exercises a fraction of LT's full rule base — and
it sets the priority order for everything below.

**Phase 1 — Core engine, in dependency order:**

1. _Morfologik FSA reader_ — load `.dict`/`.info`, perform finite-state lookups for spelling and
   morphological/POS tags. The single most important primitive; a Rust FSA/FST crate (e.g. `fst`)
   covers most of the mechanics, and the format is well-specified enough to implement reliably.
2. _Tokenizer + SRX sentence splitter_ — reproduce segmentation from the SRX rule files.
3. _POS tagger + disambiguator_ — driven by the dictionaries plus disambiguation XML; this is an
   order-dependent rule language in its own right and carries real correctness risk if mis-ordered.
4. _XML pattern-rule engine_ — parse `grammar.xml` and execute its token-pattern semantics: token
   matching, exceptions, antipatterns, `<suggestion>` generation, unification, skip/min/max
   quantifiers, filters. This is the largest single component, built out rule-feature by
   rule-feature against real rule files.
5. _Hand-coded rules_ — the Java rule classes with no XML data file must be reimplemented
   individually in Rust. Port the ones actually hit in Phase 0's scope first; treat the rest as a
   long tail to close incrementally.

**Phase 2 — Serve the HTTP contract.** A small Rust HTTP server (e.g. `axum`/`hyper`) exposing
`/v2/check` and `/v2/languages` exactly as specified above, with the CORS origin behavior. This is
the drop-in point for any consuming application.

**Phase 3 — Differential testing (the correctness oracle).** This is what makes an LLM-assisted
port trustworthy rather than a guess: run real Java LanguageTool and this engine side-by-side over
a large multi-language text corpus, diff the `matches[]` output per sentence, and close mismatches
prioritized by rule hit-frequency. Build this harness early and treat **parity percentage against
real LanguageTool** as the project's headline metric — it is a much more honest progress signal
than lines of code written or calendar time elapsed.

**Phase 4 — Integration in a consuming app.** Swap the spawned process and artifact-fetch step in
whatever app currently bundles LanguageTool's Java server; the HTTP contract means no client-side
code should need to change if Phases 1–3 hold.

## Versioning against upstream LanguageTool

Because the engine is data-driven, most upstream LanguageTool releases require **no engine code
change** — a new release's `grammar.xml` and dictionary updates can simply be dropped in and
re-validated. Occasionally a release introduces something that does require engine work:

- A new XML rule _feature_ (new attribute/filter/matching construct) the engine doesn't yet
  interpret.
- New or changed **hand-coded** rules, which have no data file and must be ported individually.

The Phase 3 differential harness doubles as the upgrade tool: point it at the new release's data
files and diff against real Java LanguageTool running that same release. The mismatch report tells
you precisely what's new-and-already-handled (free) versus what needs engine work (a sized,
measurable task) — upgrades are never a leap of faith.

It's also entirely legitimate to **pin a LanguageTool data version indefinitely** and upgrade only
when a specific new rule is wanted. Grammar rules don't go stale quickly; treat upstream tracking
as opt-in, not an obligation.

Building out the XML rule-feature set completely in Phase 1 (rather than just the subset the
initially-targeted version happens to use) minimizes the future tax — most future data drops then
"just work."

## Performance characteristics (the strongest technical case)

**Cold start.** LanguageTool's Java server is slow to become ready: JVM boot, classloading/JIT,
parsing every XML rule into a Java object graph, loading dictionaries into heap — typically
multiple seconds before the first check can run. This engine can eliminate essentially all of that
by doing the equivalent work **once, at build time** instead of on every process start:

- Precompile XML rules into a compact binary/mmappable table (rather than parsing XML at runtime).
- `mmap` the compiled rule table and FSA dictionaries so process start is "open a few files, bind a
  socket" — data pages in lazily on first touch rather than being eagerly deserialized.
- No JIT warm-up curve — full speed from the first request.

Realistic target: cold start in the tens-of-milliseconds to low-hundreds-of-milliseconds range,
versus multiple seconds for the JVM equivalent. One caveat: the very first check after a fresh
install/reboot may pay a one-time disk page-fault cost as mmap'd files first load from disk (still
fast on SSD); a small warm-up touch of the dictionary at startup removes even that if zero-latency
first-check is required. This also changes the viable process model for a consuming app — with
sub-second startup, launching the engine on first use (rather than eagerly at app launch) becomes
practical, reclaiming its resident memory entirely when unused.

**Memory.** LanguageTool's JVM process typically runs with a heap sized in the hundreds of MB
(commonly configured around 256MB–1GB), plus ~100–200MB of unavoidable JVM baseline overhead
(metaspace, JIT, GC, thread stacks) — realistically several hundred MB resident. A Rust
implementation removes the JVM baseline entirely, represents rule data in dense structs instead of
a garbage-collected object graph, and can `mmap` dictionaries and compiled rule tables so they
barely count as resident memory until touched. A resident-memory reduction on the order of 5–10×
versus the Java process is a realistic target, with steadier behavior (no GC-driven memory
sawtooth).

**Disk.** Rewriting/compiling the XML rules is primarily a _runtime_ win (parse time, memory
layout), not a disk-size win — the source XML is a small fraction of a language's data footprint
and is typically already compressed at rest. The large disk consumers are dictionary/resource data
(already FSA-compressed, with limited further headroom) and, in a bundled-JVM scenario, the JRE
itself. The dominant disk saving from this project comes from **eliminating the JVM and its JAR
dependencies entirely**, not from XML compaction.

## Scope and effort estimate

**Lines of code.** LanguageTool's full source tree is large (very roughly 500k+ lines including
all ~30+ supported languages, the full test suite, and features like neural n-gram models, cloud
integrations, and the HTTP server itself — none of which need reproducing). The actual surface to
reimplement in Rust is much smaller:

- Core engine (pattern-rule interpreter, tokenizer/tagger framework, disambiguator, check
  pipeline): roughly 40,000–80,000 lines of underlying Java behavior.
- Hand-coded rule classes across the target language set: roughly 30,000–60,000 lines of
  underlying Java behavior.
- **Total: roughly 70,000–150,000 lines of Java behavior to reproduce** — not the full upstream
  repository size, because XML rule data, the test suite, unused languages, and unused subsystems
  are excluded by construction. Note this shrinks further when actually written in idiomatic Rust,
  which is typically more compact than the equivalent Java OOP scaffolding.

**Time.** LOC count predicts code-_generation_ time (an LLM can produce this volume quickly — on
the order of weeks); it does **not** predict the calendar time to reach behavioral parity, which is
the actual bottleneck. Parity work is debugging against the differential harness — closing
thousands of individual rule/offset/ordering mismatches — not writing more code. Realistic staging,
strongly dependent on the target quality bar:

- _Plausible/MVP quality_ (reasonable results, not matched to LT): a working single-language
  checker in roughly 4–8 weeks.
- _"Users don't notice a downgrade" quality_, single language: roughly 2–4 months.
- _Full bug-for-bug parity with LanguageTool_, across a full multi-language target set: roughly
  9–18 months.

The honest recommendation is to stage delivery — reach full parity on one language first as a
proof point, driven by the differential harness's parity percentage, before committing to
additional languages.

**Error-proneness.** This is a correctness-critical reimplementation with a large behavioral
surface, so errors during development are guaranteed, not a risk to be avoided. Specific failure
modes to expect:

- Silent divergence — a rule that fires _almost_ correctly (wrong offset/length, missing or extra
  suggestion). Since offsets typically drive UI highlighting in consuming apps, small tokenization
  or Unicode-handling errors are directly user-visible.
- LLM-plausible-but-wrong code — compiles and passes obvious cases, fails subtle ones; the
  characteristic failure mode of LLM-assisted ports, and dangerous because it looks finished.
- Order-dependent disambiguation bugs that cause correct downstream rules to misfire in
  hard-to-trace ways.
- False positives (flagging correct text) cluster in whichever rules were ported last or with least
  care, and are more damaging to user trust than false negatives.

What makes this manageable rather than reckless is having a **perfect oracle**: real LanguageTool,
running the same input, is ground truth for every sentence. The differential harness converts
every one of the above failure modes from "invisible until a user reports it" into "a measurable
mismatch in a parity report" — the risk this project carries is schedule and sustained effort, not
undetectable correctness drift. The one residual risk the harness cannot resolve automatically:
occasionally LanguageTool's own behavior on some input is itself questionable, and a human has to
decide whether to match it bug-for-bug or deliberately diverge.
