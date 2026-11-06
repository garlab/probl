//! Probl compiled to WebAssembly, for the playground in the browser
//! (docs/playground.md).
//!
//! The functions take and give JSON text: [`check_program`] gives a
//! program's diagnostics, [`run`] runs it with the playground's limits, and
//! [`examples`] gives the bundled examples. A program can have several
//! files: requests give the entry's text, and the others as `"sources"`. Built for
//! `wasm32-unknown-unknown`, the module exports them as plain functions
//! (see `exports` below), and `web/src/probl.js` passes the text through
//! the module's memory. Natively, they're ordinary Rust functions, which
//! the tests call.

use probl::__internal::{EngineChecks, Symbolic, diagrams, engine_checks, loader};
use probl::{Date, FailureMode, Limits, MemoryFiles, MemorySources, Options, Sources};
use probl_sema::ir::Fault;
use probl_sema::symbols::{DefKind, Symbols};
use probl_sema::{Builtin, Source};
use probl_syntax::{Severity, SourceFile, SourceMap, Span};
use serde_json::{Value as Json, json};
use std::io::Read;
use std::sync::Arc;

/// A function told how many sampled runs are done, and how many there are.
pub type Progress = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// What a program's entry file is called, unless a request names it.
const FILE: &str = "playground.probl";

/// A program of one file's diagnostics and names: [`check_program`], for
/// `{"source": source}`.
pub fn check(source: &str) -> String {
    checked(&json!({ "source": source })).to_string()
}

/// A program's diagnostics and names. The request is `{"source": …}`, the
/// entry file's text, and optionally:
///
/// - `"name"`: what the entry is called, which its imports are resolved
///   against: `playground.probl` by default;
/// - `"sources"`: `{path: text}`, the program's other files, which it
///   imports by their paths relative to the importing file;
/// - `"file"`: the file to check, the entry by default. Another file is
///   checked as it's compiled when it's imported, with the files it imports.
///
/// The answer is `{"diagnostics": [...], "symbols": {...}, "reports":
/// [...], "files": [...]}`. Each diagnostic has its severity, message, notes
/// and help, the `file` it's about and where it is there (see
/// [`diagnostic`]), and the text the command line would print. The symbols,
/// where the checked file's names are declared and used, are there when its
/// files parse (see [`symbols`]); they're `null` otherwise. `reports` lists
/// `{id, label}` in compiled report order, or is `null` when compilation
/// fails. `files` names the files checked: the one asked for, and those it
/// imports. A request that can't be checked gets `{"error": …}`, with the
/// error's `kind` and `message`.
pub fn check_program(request: &str) -> String {
    match serde_json::from_str(request) {
        Ok(request) => checked(&request).to_string(),
        Err(e) => json!({ "error": { "kind": "internal", "message": format!("a bad request: {e}") } }).to_string(),
    }
}

fn checked(request: &Json) -> Json {
    let source = request["source"].as_str().unwrap_or_default();
    let (name, mut files) = match program_files(request, source) {
        Ok(program) => program,
        Err(message) => return json!({ "error": { "kind": "usage", "message": message } }),
    };
    let entry = files.entry(&name);
    let compiled = match request.get("file") {
        None => probl_sema::compile_program(
            Source {
                identity: entry,
                name,
                text: source.to_string(),
            },
            &mut loader(&mut files),
        ),
        Some(file) => {
            let Some(file) = file.as_str() else {
                return json!({ "error": { "kind": "usage", "message": "file must be a file's path" } });
            };
            let identity = files.entry(file);
            let text = match files.load("", &identity) {
                Ok(text) => text,
                Err(message) => {
                    let message = format!("can't check `{file}`: {message}");
                    return json!({ "error": { "kind": "usage", "message": message } });
                }
            };
            let source = Source {
                identity: identity.clone(),
                name: text.name().to_string(),
                text: text.text().to_string(),
            };
            if identity == entry {
                probl_sema::compile_program(source, &mut loader(&mut files))
            } else {
                probl_sema::compile_module(source, &mut loader(&mut files))
            }
        }
    };
    let reports = compiled.program.as_ref().map(|p| {
        p.reports
            .iter()
            .enumerate()
            .map(|(id, r)| json!({"id":id, "label":r.label}))
            .collect::<Vec<_>>()
    });
    let symbols = compiled.symbols.map(|s| self::symbols(&s, &compiled.sources));
    let diagnostics: Vec<Json> = compiled
        .diagnostics
        .iter()
        .map(|d| compiler_diagnostic(d, &compiled.sources))
        .collect();
    let checked: Vec<&str> = compiled.sources.files().map(|(_, f)| f.name.as_str()).collect();
    json!({ "diagnostics": diagnostics, "symbols": symbols, "reports": reports, "files": checked })
}

/// A request's program: what its entry is called, and its files, the entry
/// with `source` among them.
fn program_files(request: &Json, source: &str) -> Result<(String, MemorySources), String> {
    let name = match request.get("name") {
        None => FILE,
        Some(name) => name
            .as_str()
            .filter(|n| !n.is_empty())
            .ok_or("name must be the entry file's path")?,
    };
    let mut files = MemorySources::new();
    if let Some(sources) = request.get("sources") {
        let sources = sources
            .as_object()
            .ok_or("sources must be an object of paths and texts")?;
        for (path, text) in sources {
            let text = text
                .as_str()
                .ok_or_else(|| format!("the source of `{path}` must be text"))?;
            files.insert(path.as_str(), text);
        }
    }
    files.insert(name, source);
    Ok((name.to_string(), files))
}

/// Offsets in UTF-16 code units, as JavaScript counts them, for byte offsets
/// in a text.
struct Utf16(Option<Vec<u32>>);

impl Utf16 {
    fn new(text: &str) -> Utf16 {
        if text.is_ascii() {
            return Utf16(None);
        }
        let mut table = vec![0; text.len() + 1];
        let mut units = 0;
        for (i, c) in text.char_indices() {
            table[i..i + c.len_utf8()].fill(units);
            units += c.len_utf16() as u32;
        }
        table[text.len()] = units;
        Utf16(Some(table))
    }

    fn at(&self, byte: u32) -> u32 {
        match &self.0 {
            None => byte,
            Some(table) => table[(byte as usize).min(table.len() - 1)],
        }
    }
}

/// A file's text, where it is in a [`SourceMap`], and how to count in it.
struct Text<'a> {
    name: &'a str,
    text: &'a str,
    base: u32,
    utf16: Utf16,
    newlines: Vec<usize>,
}

impl Text<'_> {
    /// A map offset in this file, in UTF-16 code units from its start.
    fn at(&self, offset: u32) -> u32 {
        self.utf16.at(offset.saturating_sub(self.base))
    }

    /// Whether a map offset is in this file, or at its end.
    fn has(&self, offset: u32) -> bool {
        self.base <= offset && offset <= self.base + self.text.len() as u32
    }
}

/// The checked file's names, for an editor. The checked file is the first
/// in `sources`, and the others are the files it imports.
///
/// - `definitions`: each with its name, kind, where it's declared (`from`,
///   `to`, `line`), where it can be used (`scope`, and `global` for
///   top-level variables, which functions can also use), its declaration's
///   line (`detail`), and the comments that describe it (`doc`): the lines
///   of comments just above, and a comment at the end of its line. A name
///   an import declares has the index of the definition it imports, in
///   `imported`. Definitions in another file, which the checked file uses
///   (imported ones, and their fields and variants), say which `file`,
///   with their `from`, `to` and `line` in it, and a `scope` of `[-1, -1]`:
///   they can't be used by name in the checked file;
/// - `references`: `[from, to, definition]` for every use of a name;
/// - `functions`: `[from, to]` for each named function.
///
/// Offsets count UTF-16 code units.
fn symbols(symbols: &Symbols, sources: &SourceMap) -> Json {
    let texts: Vec<Text> = sources
        .files()
        .map(|(base, file)| Text {
            name: &file.name,
            text: &file.text,
            base,
            utf16: Utf16::new(&file.text),
            newlines: file.text.match_indices('\n').map(|(i, _)| i).collect(),
        })
        .collect();
    let root = &texts[0];
    let file_of = |offset: u32| sources.file_index(offset).unwrap_or(0);
    // The checked file's definitions, then the others it uses, in order.
    let mut index: Vec<Option<usize>> = vec![None; symbols.definitions.len()];
    let mut order = Vec::new();
    for (i, d) in symbols.definitions.iter().enumerate() {
        if root.has(d.span.lo) {
            index[i] = Some(order.len());
            order.push(i);
        }
    }
    let used = symbols
        .references
        .iter()
        .filter(|(span, _)| root.has(span.lo))
        .map(|&(_, d)| d)
        .chain(order.iter().filter_map(|&i| symbols.definitions[i].imported))
        .collect::<Vec<_>>();
    for d in used {
        if index[d].is_none() {
            index[d] = Some(order.len());
            order.push(d);
        }
    }
    let definitions: Vec<Json> = order
        .iter()
        .map(|&i| {
            let d = &symbols.definitions[i];
            let file = file_of(d.span.lo);
            let t = &texts[file];
            let at = (d.span.lo - t.base) as usize;
            let (line, detail, doc) = declaration(t.text, &t.newlines, at);
            let detail = match (d.kind, &d.owner, &d.ty) {
                (DefKind::Field, Some(owner), Some(ty)) => format!("{owner}.{}: {ty}", d.name),
                _ => detail,
            };
            let elsewhere = file != 0;
            let scope = if elsewhere {
                json!([-1, -1])
            } else {
                let end = root.base + root.text.len() as u32;
                json!([
                    root.at(d.scope.lo.clamp(root.base, end)),
                    root.at(d.scope.hi.clamp(root.base, end))
                ])
            };
            json!({
                "name": d.name,
                "kind": d.kind.name(),
                "mutable": d.mutable,
                "global": d.global && !elsewhere,
                "owner": d.owner,
                "from": t.at(d.span.lo),
                "to": t.at(d.span.hi),
                "line": line,
                "scope": scope,
                "detail": detail,
                "doc": doc,
                "file": elsewhere.then_some(t.name),
                "imported": d.imported.and_then(|origin| index[origin]),
            })
        })
        .collect();
    let references: Vec<Json> = symbols
        .references
        .iter()
        .filter(|(span, _)| root.has(span.lo))
        .filter_map(|&(span, d)| Some(json!([root.at(span.lo), root.at(span.hi), index[d]?])))
        .collect();
    let functions: Vec<Json> = symbols
        .functions
        .iter()
        .filter(|f| root.has(f.lo))
        .map(|f| json!([root.at(f.lo), root.at(f.hi)]))
        .collect();
    json!({ "definitions": definitions, "references": references, "functions": functions })
}

/// For a declaration at byte `at`: its line number (from 1), its line
/// without a comment at the end or an opening brace, and the comments that
/// describe it. `newlines` are where the source's lines end.
fn declaration(source: &str, newlines: &[usize], at: usize) -> (usize, String, Option<String>) {
    let at = at.min(source.len());
    let start = source[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = source[at..].find('\n').map_or(source.len(), |i| at + i);
    let line = &source[start..end];
    let (code, trailing) = split_comment(line);
    let mut code = code.trim().trim_end_matches('{').trim_end().to_string();
    if code.chars().count() > 120 {
        code = code.chars().take(119).collect::<String>() + "…";
    }
    // Comment lines just above, nearest last.
    let mut doc: Vec<&str> = source[..start]
        .lines()
        .rev()
        .take_while(|l| l.trim_start().starts_with('#'))
        .map(|l| l.trim_start().trim_start_matches('#').trim())
        .collect();
    doc.reverse();
    doc.extend(trailing);
    let doc = (!doc.is_empty()).then(|| doc.join("\n"));
    let number = newlines.partition_point(|&i| i < start) + 1;
    (number, code, doc)
}

/// A line's code, and its comment at the end, if it has one: a `#` that
/// isn't in a string.
fn split_comment(line: &str) -> (&str, Option<&str>) {
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in line.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return (&line[..i], Some(line[i + 1..].trim())),
            _ => {}
        }
    }
    (line, None)
}

/// The reference: `{"builtins": [...], "constants": [...], "keywords": [...],
/// "faults": [...], "read": {...}}`, each with its `signature` and `summary`,
/// and the built-ins with their `category`. The faults are what a `catch`
/// can name.
pub fn docs() -> String {
    let builtins: Vec<Json> = Builtin::ALL
        .iter()
        .filter(|b| b.is_public())
        .filter_map(|&b| {
            let d = probl_sema::docs::builtin(b)?;
            Some(json!({
                "name": b.name(),
                "category": probl_sema::docs::category(b),
                "signature": d.signature,
                "summary": d.summary,
            }))
        })
        .collect();
    let keywords: Vec<Json> = probl_sema::docs::KEYWORDS
        .iter()
        .filter_map(|&k| {
            let d = probl_sema::docs::keyword(k)?;
            Some(json!({ "name": k, "signature": d.signature, "summary": d.summary }))
        })
        .collect();
    let constants: Vec<Json> = probl_sema::Constant::ALL
        .iter()
        .map(|&c| {
            let d = probl_sema::docs::constant(c);
            json!({ "name": c.name(), "signature": d.signature, "summary": d.summary })
        })
        .collect();
    let faults: Vec<Json> = Fault::ALL
        .iter()
        .map(|&f| {
            let d = probl_sema::docs::fault(f);
            json!({ "name": f.name(), "signature": d.signature, "summary": d.summary })
        })
        .collect();
    let read = probl_sema::docs::READ;
    json!({
        "builtins": builtins,
        "constants": constants,
        "keywords": keywords,
        "faults": faults,
        "read": { "name": "read", "signature": read.signature, "summary": read.summary },
    })
    .to_string()
}

/// Run a program. The request is `{"source": …}`, and optionally:
///
/// - `"name"` and `"sources"`: what the entry file is called, and the
///   program's other files, as for [`check_program`];
/// - `"mode"`: `"enumerate"` or `"sample"`, and `"runs"` and `"seed"`,
///   which override the program's `@mode` as on the command line;
/// - `"conjugate"`: `false` to sample without exact updates;
/// - `"report_collections"`: report ID strings mapped to "exact", "moments" or "quantiles[:CAPACITY]".
/// - `"report_summaries"`: an object from zero-based report ID strings to summary
///   specifications, such as `{"0":"forecast","2":"mean,median,q:0.9"}`;
/// - `"on_error"`: `"total"` or `"partial"`, overriding the program's
///   `@on_error` as `--on-error` does;
/// - `"files"`: `{path: text}`, the files `read` may read.
/// - `"today"`: `"YYYY-MM-DD"`, the date captured by the host for this execution.
///   Alone, this also pins `now` to midnight.
/// - `"now"`: a datetime string, optionally with an offset; derives `today`.
///   When both are supplied, normalized dates must agree. The JavaScript wrapper
///   captures UTC once when neither is given.
/// - `"env"`: `{name: text}`, the environment that `env` stands for. The
///   module has no environment of its own, and the playground sends none.
/// - `"symbolic"`: `"only"` or `"prefer"`, to answer the program with
///   decision diagrams, as `probl run --symbolic` does. Experimental, for
///   checking the engine: the playground doesn't send it. The stats then
///   have `"variables"` and `"nodes"`.
///
/// `print` receives what the program prints, as it prints it, and
/// `progress`, when sampling, how many runs are done after each batch, and
/// how many there are. The answer is
/// `{"output": …, "stats": …, "diagnostics": [...]}`, or
/// `{"error": …, "diagnostics": [...]}`, with the error described as a
/// diagnostic, and its `"kind"`: `"usage"`, `"language"`, `"unsupported"`, `"limit"`
/// or `"internal"`. A partial result, where some worlds failed, has both:
/// `{"output": …, "stats": …, "partial": true, "error": …, "failures":
/// [...], "diagnostics": [...]}`, with a diagnostic for each place they
/// failed.
/// Successful and partial results also include a versioned `reports` document,
/// or `reports_error` when its independent serialization budgets were exceeded.
/// That error does not invalidate the model's output or replace its diagnostics.
pub fn run(request: &str, print: &mut (dyn FnMut(&str) + Send), progress: Option<Progress>) -> String {
    let request: Json = match serde_json::from_str(request) {
        Ok(r) => r,
        Err(e) => {
            return json!({ "error": { "kind": "internal", "message": format!("a bad request: {e}") } }).to_string();
        }
    };
    let source = request["source"].as_str().unwrap_or_default();
    let (name, mut sources) = match program_files(&request, source) {
        Ok(program) => program,
        Err(message) => return json!({ "error": { "kind": "usage", "message": message } }).to_string(),
    };
    let program = match probl::compile_with(&name, source, &mut sources) {
        Ok(program) => program,
        Err(e) => {
            let diagnostics: Vec<Json> = e.diagnostics().iter().map(diagnostic).collect();
            let first = e
                .diagnostics()
                .iter()
                .find(|d| d.severity() == probl::Severity::Error)
                .map(diagnostic);
            return json!({ "error": first, "diagnostics": diagnostics }).to_string();
        }
    };
    let diagnostics: Vec<Json> = program.warnings().iter().map(diagnostic).collect();
    // An error in the request, about no place in the program.
    let entry = SourceMap::single(SourceFile::new(name.as_str(), source));
    let invalid = |message: &str| {
        let mut error = compiler_diagnostic(&probl_syntax::Diagnostic::error(Span::default(), message), &entry);
        error["kind"] = json!("language");
        error
    };
    let failed = |error: Json| json!({ "error": error, "diagnostics": diagnostics }).to_string();
    let mut options = mode(&request, Options::new())
        .limits(Limits::browser())
        .conjugate(request["conjugate"].as_bool().unwrap_or(true));
    let overridden = overrides::<probl::ReportSummary>(&request, "report_summaries", "summary").and_then(|summaries| {
        Ok((
            summaries,
            overrides::<probl::ReportCollection>(&request, "report_collections", "collection")?,
        ))
    });
    let (summaries, collections) = match overridden {
        Ok(overridden) => overridden,
        Err(message) => return failed(json!({"kind":"usage", "message":message})),
    };
    for (id, summary) in summaries {
        options = options.report_summary(id, summary);
    }
    for (id, collection) in collections {
        options = options.report_collection(id, collection);
    }
    let symbolic = match request["symbolic"].as_str() {
        Some("only") => Symbolic::Only,
        Some("prefer") => Symbolic::Prefer,
        _ => Symbolic::Off,
    };
    if symbolic != Symbolic::Off {
        let checks = EngineChecks {
            symbolic,
            ..EngineChecks::default()
        };
        options = engine_checks(options, checks);
    }
    if let Some(progress) = progress {
        options = options.progress(move |done, total| progress(done, total));
    }
    if let Some(today) = request.get("today") {
        match today.as_str().and_then(|s| s.parse::<Date>().ok()) {
            Some(date) => options = options.today(date),
            None => {
                return failed(invalid("today must be YYYY-MM-DD within 0001-01-01..9999-12-31"));
            }
        }
    }
    if let Some(now) = request.get("now") {
        match now.as_str().and_then(|s| s.parse::<probl::DateTime>().ok()) {
            Some(datetime) => options = options.now(datetime),
            None => {
                return failed(invalid(
                    "now must be a datetime within years 1..9999, with optional fractional seconds and UTC offset",
                ));
            }
        }
    }
    if let Some(env) = request.get("env") {
        let vars = env.as_object().and_then(|vars| {
            vars.iter()
                .map(|(name, text)| Some((name, text.as_str()?)))
                .collect::<Option<Vec<_>>>()
        });
        let Some(vars) = vars else {
            return failed(invalid("env must be an object whose values are strings"));
        };
        for (name, text) in vars {
            options = options.env(name, text);
        }
    }
    if program.reads_data() {
        let given = request["files"].as_object().cloned().unwrap_or_default();
        let mut files = Files(
            given
                .iter()
                .map(|(path, text)| (path.clone(), text.as_str().unwrap_or_default()))
                .collect(),
        );
        match program.load(&mut files, &options) {
            Ok(data) => options = options.data(data),
            Err(e) => return failed(error(&e)),
        }
    }
    let stats = |outcome: &probl::Outcome| {
        let s = outcome.stats();
        let mut stats = json!({
            "peak_worlds": s.peak_worlds(),
            "world_steps": s.world_steps(),
            "calls": s.calls(),
            "reused_calls": s.reused_calls(),
            "solved_loops": s.solved_loops(),
            "chain_states": s.chain_states(),
            "solved_calls": s.solved_calls(),
            "call_rounds": s.call_rounds(),
        });
        if let Some(sampled) = outcome.sampling() {
            stats["runs"] = json!(sampled.runs());
            stats["effective_runs"] = json!(sampled.effective_runs());
        }
        if let Some((variables, nodes)) = diagrams(s) {
            stats["variables"] = json!(variables);
            stats["nodes"] = json!(nodes);
        }
        stats
    };
    let with_reports = |mut response: Json, outcome: &probl::Outcome| {
        match outcome.reports_json(&probl::ReportJsonLimits::default()) {
            Ok(document) => response["reports"] = serde_json::from_str(&document).expect("library report JSON"),
            Err(e) => response["reports_error"] = json!({"kind":"limit", "message":e.to_string()}),
        }
        response.to_string()
    };
    match program.run_with(&options, print) {
        Ok(outcome) => {
            let today = outcome.today().map(|d| d.to_string());
            let now = outcome.now().map(|d| d.to_string());
            with_reports(
                json!({ "output": outcome.text(), "today": today, "now": now, "stats": stats(&outcome), "diagnostics": diagnostics }),
                &outcome,
            )
        }
        Err(e) if e.partial().is_some() => {
            let outcome = e.partial().expect("checked");
            let today = outcome.today().map(|d| d.to_string());
            let now = outcome.now().map(|d| d.to_string());
            let failures: Vec<Json> = e.diagnostics().iter().map(diagnostic).collect();
            with_reports(
                json!({
                    "output": outcome.text(),
                    "today": today, "now": now,
                    "stats": stats(outcome),
                    "partial": true,
                    "finished": outcome.finished(),
                    "error": error(&e),
                    "failures": failures,
                    "diagnostics": diagnostics,
                }),
                outcome,
            )
        }
        Err(e) => failed(error(&e)),
    }
}

/// The options a request asks for, as the command line's `--mode`, `--runs`,
/// `--seed` and `--on-error` would.
fn mode(request: &Json, options: Options) -> Options {
    let mut options = match request["mode"].as_str() {
        Some("enumerate") => options.enumerate(),
        Some("sample") => options.sample(),
        _ => options,
    };
    if let Some(runs) = request["runs"].as_u64() {
        options = options.runs(runs);
    }
    if let Some(seed) = request["seed"].as_u64() {
        options = options.seed(seed);
    }
    match request["on_error"].as_str() {
        Some("total") => options.on_error(FailureMode::Total),
        Some("partial") => options.on_error(FailureMode::Partial),
        _ => options,
    }
}

/// A request's per-report overrides in `field`, `{"ID": "SPEC"}`, each SPEC
/// parsed as a `T`: at most 256, with canonical zero-based IDs. `what` names
/// them in errors. Without the field there are none.
fn overrides<T: std::str::FromStr>(request: &Json, field: &str, what: &str) -> Result<Vec<(usize, T)>, String>
where
    T::Err: std::fmt::Display,
{
    let Some(entries) = request.get(field) else {
        return Ok(Vec::new());
    };
    let entries = entries
        .as_object()
        .ok_or_else(|| format!("{field} must be an object of report IDs and {what} strings"))?;
    if entries.len() > 256 {
        return Err(format!("at most 256 report {what} overrides are allowed"));
    }
    entries
        .iter()
        .map(|(id, spec)| {
            let index: usize = id.parse().map_err(|_| format!("invalid report {what} ID `{id}`"))?;
            if id != &index.to_string() {
                return Err(format!(
                    "invalid report {what} ID `{id}`; use a canonical zero-based integer"
                ));
            }
            let spec = spec
                .as_str()
                .ok_or_else(|| format!("report {what} {id} must be a string"))?;
            let parsed = spec.parse().map_err(|e| format!("report {what} {id}: {e}"))?;
            Ok((index, parsed))
        })
        .collect()
}

/// The files a program may read: only those given, by the path it's
/// written with.
struct Files(MemoryFiles);

impl probl::Files for Files {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        if path == "-" {
            return Err("the playground has no standard input".into());
        }
        if !self.0.contains(path) {
            return Err(format!("`{path}` isn't one of the playground's files"));
        }
        self.0.resolve(path)
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        self.0.open(identity)
    }
}

/// A diagnostic as JSON, with the `file` it's about. `from` and `to` count
/// UTF-16 code units in that file, as JavaScript strings do; `line` and
/// `column` start at 1.
fn diagnostic(d: &probl::Diagnostic) -> Json {
    let severity = match d.severity() {
        probl::Severity::Warning => "warning",
        _ => "error",
    };
    let span = d.span();
    let fields = Fields {
        file: d.file(),
        from: utf16(d.file_text(), span.start),
        to: utf16(d.file_text(), span.end),
        line_column: d.line_column(),
        severity,
        message: d.message(),
        notes: d.notes(),
        help: d.help(),
    };
    fields.json(d.render(false))
}

/// A diagnostic of the compiler's, whose span is in `sources`' offsets, for
/// the editor, as JSON (see [`diagnostic`]).
fn compiler_diagnostic(d: &probl_syntax::Diagnostic, sources: &SourceMap) -> Json {
    let severity = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let mut d = d.clone();
    let file = match sources.locate(d.span) {
        Some((file, span)) => {
            d.span = span;
            file.clone()
        }
        None => Arc::new(SourceFile::new("", "")),
    };
    let (lo, hi) = (d.span.lo as usize, d.span.hi.max(d.span.lo) as usize);
    let fields = Fields {
        file: &file.name,
        from: utf16(&file.text, lo),
        to: utf16(&file.text, hi),
        line_column: file.line_col(clamp(&file.text, lo) as u32),
        severity,
        message: &d.message,
        notes: &d.notes,
        help: d.help.as_deref(),
    };
    fields.json(d.render(&file, false))
}

/// What a diagnostic's JSON says.
struct Fields<'a> {
    file: &'a str,
    from: usize,
    to: usize,
    line_column: (usize, usize),
    severity: &'a str,
    message: &'a str,
    notes: &'a [String],
    help: Option<&'a str>,
}

impl Fields<'_> {
    fn json(self, rendered: String) -> Json {
        let (line, column) = self.line_column;
        json!({
            "file": self.file,
            "from": self.from,
            "to": self.to,
            "line": line,
            "column": column,
            "severity": self.severity,
            "message": self.message,
            "notes": self.notes,
            "help": self.help,
            "rendered": rendered,
        })
    }
}

/// An error, as a diagnostic with its kind.
fn error(e: &probl::Error) -> Json {
    let mut out = e
        .diagnostics()
        .first()
        .map_or_else(|| json!({ "message": e.to_string() }), diagnostic);
    out["kind"] = json!(match e.kind() {
        probl::ErrorKind::Unsupported => "unsupported",
        probl::ErrorKind::Limit => "limit",
        probl::ErrorKind::Internal => "internal",
        probl::ErrorKind::Usage => "usage",
        _ => "language",
    });
    out
}

/// `at`, moved back to the start of the character it's in.
fn clamp(text: &str, at: usize) -> usize {
    let mut i = at.min(text.len());
    while !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// A byte offset in `text` as UTF-16 code units.
fn utf16(text: &str, at: usize) -> usize {
    text[..clamp(text, at)].encode_utf16().count()
}

/// The bundled examples: `[{"name", "title", "source", "files"}]`, the
/// title from the first comment, and `files` the data they read.
pub fn examples() -> String {
    macro_rules! example {
        ($name:literal) => {
            (
                $name,
                include_str!(concat!("../../../examples/", $name, ".probl")),
            )
        };
    }
    let all = [
        example!("01_tour"),
        example!("02_craps"),
        example!("03_rpg_duel"),
        example!("04_risk_battle"),
        example!("05_snakes_and_ladders"),
        example!("06_blackjack_dealer"),
        example!("07_launch_forecast"),
        example!("08_signup_forecast"),
        example!("09_roadmap"),
        example!("10_quantum_key"),
        example!("11_delivery_dates"),
        example!("12_invoice_calendar"),
        example!("13_renewal_dates"),
        example!("14_stock_decision"),
        example!("15_service_queue"),
        example!("16_predictive_check"),
        example!("17_sensor_tracking"),
        example!("18_correlated_losses"),
        example!("19_analytic_continuous"),
        example!("20_set"),
        example!("21_retirement"),
        example!("22_correlated_returns"),
    ];
    let data = json!({ "data/pilot.csv": include_str!("../../../examples/data/pilot.csv") });
    let list: Vec<Json> = all
        .iter()
        .map(|(name, source)| {
            let title = source
                .lines()
                .find_map(|l| l.strip_prefix("# "))
                .unwrap_or(name)
                .trim_end_matches('.');
            let files = if source.contains("read(\"data/") {
                data.clone()
            } else {
                json!({})
            };
            json!({ "name": name, "title": title, "source": source, "files": files })
        })
        .collect();
    Json::Array(list).to_string()
}

/// The exported functions, for `wasm32-unknown-unknown`. Text goes in
/// through memory the module allocates (`probl_alloc`), and comes back in a
/// buffer the module keeps until the next call (`probl_result`), whose
/// length each function returns.
#[cfg(target_arch = "wasm32")]
mod exports {
    use std::cell::RefCell;
    use std::sync::Once;

    #[link(wasm_import_module = "probl")]
    unsafe extern "C" {
        /// A line the program printed.
        fn print(ptr: *const u8, len: usize);
        /// What a panic said, just before the module stops.
        fn panicked(ptr: *const u8, len: usize);
        /// Runs done, and runs there are, after each batch of sampled runs.
        fn progress(done: f64, total: f64);
    }

    thread_local! {
        static RESULT: RefCell<String> = const { RefCell::new(String::new()) };
    }

    static HOOK: Once = Once::new();

    fn setup() {
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| {
                let message = info.to_string();
                // SAFETY: the pointer and length describe `message`.
                unsafe { panicked(message.as_ptr(), message.len()) }
            }));
        });
    }

    /// Take the text the caller wrote into memory from `probl_alloc`.
    fn take(ptr: *mut u8, len: usize) -> String {
        // SAFETY: `probl_alloc(len)` returned `ptr`, and it's taken once.
        let bytes = unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) };
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn give(text: String) -> usize {
        let len = text.len();
        RESULT.with(|r| *r.borrow_mut() = text);
        len
    }

    /// Memory for `len` bytes of text, taken back by the call it's given to.
    #[unsafe(no_mangle)]
    pub extern "C" fn probl_alloc(len: usize) -> *mut u8 {
        Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
    }

    /// The text the last call gave back.
    #[unsafe(no_mangle)]
    pub extern "C" fn probl_result() -> *const u8 {
        RESULT.with(|r| r.borrow().as_ptr())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_check(ptr: *mut u8, len: usize) -> usize {
        setup();
        give(super::check_program(&take(ptr, len)))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_run(ptr: *mut u8, len: usize) -> usize {
        setup();
        let request = take(ptr, len);
        // SAFETY: the pointer and length describe `line`.
        let mut printed = |line: &str| unsafe { print(line.as_ptr(), line.len()) };
        // SAFETY: `progress` takes two numbers.
        let told: super::Progress = std::sync::Arc::new(|done, total| unsafe { progress(done as f64, total as f64) });
        give(super::run(&request, &mut printed, Some(told)))
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_examples() -> usize {
        setup();
        give(super::examples())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_docs() -> usize {
        setup();
        give(super::docs())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn probl_version() -> usize {
        give(env!("CARGO_PKG_VERSION").to_string())
    }
}
