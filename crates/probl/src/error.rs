//! What can go wrong: errors, and the diagnostics that describe them.

use crate::Outcome;
use probl_engine::RuntimeError;
use probl_syntax::{SourceFile, Span};
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// What kind of problem an [`Error`] is.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// The program didn't compile. The diagnostics have every error and
    /// warning.
    Compile,
    /// The host misused the library: it ran a program that reads data
    /// without its data, or with data loaded for another program.
    Usage,
    /// The program did something the language doesn't allow.
    Language,
    /// The program uses a feature that isn't built yet.
    Unsupported,
    /// The run reached a limit, or was cancelled.
    Limit,
    /// A bug in Probl.
    Internal,
}

/// How serious a [`Diagnostic`] is.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// An error or a warning about a place in a program. It keeps the program's
/// source, so it can render itself.
#[derive(Clone)]
pub struct Diagnostic {
    inner: probl_syntax::Diagnostic,
    file: Arc<SourceFile>,
}

impl Diagnostic {
    pub(crate) fn new(inner: probl_syntax::Diagnostic, file: &Arc<SourceFile>) -> Diagnostic {
        Diagnostic {
            inner,
            file: file.clone(),
        }
    }

    pub fn severity(&self) -> Severity {
        match self.inner.severity {
            probl_syntax::Severity::Error => Severity::Error,
            probl_syntax::Severity::Warning => Severity::Warning,
        }
    }

    pub fn message(&self) -> &str {
        &self.inner.message
    }

    pub fn notes(&self) -> &[String] {
        &self.inner.notes
    }

    pub fn help(&self) -> Option<&str> {
        self.inner.help.as_deref()
    }

    /// Where it is in the source, in bytes.
    pub fn span(&self) -> Range<usize> {
        let len = self.file.text.len();
        let lo = (self.inner.span.lo as usize).min(len);
        lo..(self.inner.span.hi as usize).clamp(lo, len)
    }

    /// The line and column where it starts, both from 1.
    pub fn line_column(&self) -> (usize, usize) {
        self.file.line_col(self.inner.span.lo)
    }

    /// As the command line prints it, in color or not.
    pub fn render(&self, color: bool) -> String {
        self.inner.render(&self.file, color)
    }
}

impl fmt::Debug for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagnostic")
            .field("severity", &self.severity())
            .field("message", &self.message())
            .field("span", &self.span())
            .finish()
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

/// What a fault that stopped a run says about itself: the failing
/// operation's inputs, and how often it fails. Every part is optional: an
/// absent one couldn't be established, which doesn't mean zero. Its
/// [`Error`]'s diagnostic has the same as a note.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultContext {
    inputs: Vec<FaultInput>,
    share: Option<f64>,
    per_pass: bool,
    run: Option<FailedRun>,
}

impl FaultContext {
    /// The operation's inputs other than literals, as the world or run that
    /// failed had them.
    pub fn inputs(&self) -> &[FaultInput] {
        &self.inputs
    }

    /// When enumerating: of the weight reaching the operation, the share for
    /// which it fails. That includes the evidence so far; it isn't the
    /// probability that the program fails.
    pub fn share(&self) -> Option<f64> {
        self.share
    }

    /// Whether [`share`](Self::share) counts only the loop's current pass.
    pub fn per_loop_pass(&self) -> bool {
        self.per_pass
    }

    /// When sampling: the run that failed.
    pub fn run(&self) -> Option<FailedRun> {
        self.run
    }
}

/// An input of a failing operation, in a [`FaultContext`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultInput {
    name: Option<String>,
    source: String,
    value: String,
    span: Range<usize>,
}

impl FaultInput {
    /// The variable's name, when the input is one.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Its source text.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Its value, as text, cut short with `…` when long.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Where it is in the source, in bytes.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }
}

/// The sampled run in which a fault stopped the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailedRun {
    run: u64,
    runs: u64,
    seed: u64,
}

impl FailedRun {
    /// The run, from 1.
    pub fn run(&self) -> u64 {
        self.run
    }

    /// How many runs were requested.
    pub fn runs(&self) -> u64 {
        self.runs
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }
}

impl FaultContext {
    fn new(c: &probl_engine::FaultContext, file: &SourceFile) -> FaultContext {
        let len = file.text.len();
        let inputs = c
            .inputs
            .iter()
            .map(|input| {
                let lo = (input.span.lo as usize).min(len);
                let span = lo..(input.span.hi as usize).clamp(lo, len);
                FaultInput {
                    name: input.name.clone(),
                    source: file.text.get(span.clone()).unwrap_or_default().to_string(),
                    value: input.value.clone(),
                    span,
                }
            })
            .collect();
        FaultContext {
            inputs,
            share: c.share,
            per_pass: c.per_pass,
            run: c.run.map(|r| FailedRun {
                run: r.run + 1,
                runs: r.runs,
                seed: r.seed,
            }),
        }
    }

    /// The note that says it: `fails for 16.67% of the weight reaching this
    /// operation, for example when x = 0`.
    fn note(&self) -> Option<String> {
        // A variable by its name, anything else by its source text when short.
        let inputs: Vec<String> = self
            .inputs
            .iter()
            .filter_map(|i| {
                let source = unparenthesized(&i.source);
                let label = match &i.name {
                    Some(name) => name.clone(),
                    None if !source.is_empty() && source.len() <= 40 && !source.contains('\n') => format!("`{source}`"),
                    None => return None,
                };
                Some(format!("{label} = {}", i.value))
            })
            .collect();
        let when = match inputs.split_last() {
            None => None,
            Some((last, [])) => Some(last.clone()),
            Some((last, rest)) => Some(format!("{} and {last}", rest.join(", "))),
        };
        let thousands = |n: u64| probl_engine::report::thousands(i64::try_from(n).unwrap_or(i64::MAX));
        Some(match (&self.run, self.share, when) {
            (Some(r), _, when) => {
                let mut note = format!(
                    "fails in run {} of {} (seed {})",
                    thousands(r.run),
                    thousands(r.runs),
                    r.seed
                );
                if let Some(when) = when {
                    note.push_str(&format!(", when {when}"));
                }
                note
            }
            (None, Some(share), when) => {
                let mut note = format!(
                    "fails for {} of the weight reaching this operation",
                    probl_engine::value::fmt_prob(share)
                );
                if self.per_pass {
                    note.push_str(" on this pass through the loop");
                }
                if let Some(when) = when {
                    note.push_str(&format!(", for example when {when}"));
                }
                note
            }
            (None, None, Some(when)) => format!("for example, it fails when {when}"),
            (None, None, None) => return None,
        })
    }
}

/// `s` without the parentheses around all of it: `(x - 2)` is `x - 2`.
fn unparenthesized(s: &str) -> &str {
    let mut s = s.trim();
    loop {
        let mut depth = 0;
        let mut wrapped = false;
        for (i, c) in s.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        wrapped = i == s.len() - 1 && s.starts_with('(');
                        break;
                    }
                }
                _ if depth == 0 => break,
                _ => {}
            }
        }
        if !wrapped {
            return s;
        }
        s = s[1..s.len() - 1].trim();
    }
}

/// A problem compiling a program, loading its data or running it.
#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    diagnostics: Vec<Diagnostic>,
    partial: Option<Arc<Outcome>>,
    fault: Option<Arc<FaultContext>>,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// When compiling, every diagnostic; otherwise, the one that describes
    /// the problem.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// As the command line prints it, in color or not.
    pub fn render(&self, color: bool) -> String {
        self.diagnostics.iter().map(|d| d.render(color)).collect()
    }

    /// When a run in partial mode had worlds that failed: what the others
    /// gave, with its [`failures`](Outcome::failures). The error has a
    /// diagnostic for each place they failed (the first ten). `None` for
    /// every other error.
    pub fn partial(&self) -> Option<&Outcome> {
        self.partial.as_deref()
    }

    /// When a fault stopped the run: its operation's inputs and how often it
    /// fails, as far as they could be established. `None` for every other
    /// error, and for a fault with nothing to add.
    pub fn fault(&self) -> Option<&FaultContext> {
        self.fault.as_deref()
    }

    pub(crate) fn partial_mut(&mut self) -> Option<&mut Outcome> {
        self.partial.as_mut().and_then(Arc::get_mut)
    }

    /// A partial result: some worlds failed.
    pub(crate) fn failed(outcome: Outcome) -> Error {
        Error {
            kind: ErrorKind::Language,
            diagnostics: outcome
                .failures()
                .iter()
                .take(10)
                .map(|f| f.diagnostic().clone())
                .collect(),
            partial: Some(Arc::new(outcome)),
            fault: None,
        }
    }

    pub(crate) fn compile(diagnostics: Vec<probl_syntax::Diagnostic>, file: &Arc<SourceFile>) -> Error {
        Error {
            kind: ErrorKind::Compile,
            diagnostics: diagnostics.into_iter().map(|d| Diagnostic::new(d, file)).collect(),
            partial: None,
            fault: None,
        }
    }

    /// An error that stopped a run. A fault's context becomes a note.
    pub(crate) fn runtime(e: RuntimeError, file: &Arc<SourceFile>) -> Error {
        let kind = match e.kind {
            probl_engine::ErrorKind::Language => ErrorKind::Language,
            probl_engine::ErrorKind::Unsupported => ErrorKind::Unsupported,
            probl_engine::ErrorKind::Limit => ErrorKind::Limit,
            probl_engine::ErrorKind::Internal => ErrorKind::Internal,
        };
        let fault = e
            .context
            .as_deref()
            .filter(|_| e.fault.is_some())
            .map(|c| FaultContext::new(c, file));
        let mut diagnostic = e.to_diagnostic();
        if let Some(note) = fault.as_ref().and_then(FaultContext::note) {
            diagnostic.notes.push(note);
        }
        Error {
            kind,
            diagnostics: vec![Diagnostic::new(diagnostic, file)],
            partial: None,
            fault: fault.map(Arc::new),
        }
    }

    pub(crate) fn usage(span: Span, message: &str, help: &str, file: &Arc<SourceFile>) -> Error {
        Error {
            kind: ErrorKind::Usage,
            diagnostics: vec![Diagnostic::new(
                probl_syntax::Diagnostic::error(span, message).with_help(help),
                file,
            )],
            partial: None,
            fault: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let first = self
            .diagnostics
            .iter()
            .find(|d| d.severity() == Severity::Error)
            .or(self.diagnostics.first());
        match first {
            Some(d) => f.write_str(d.message()),
            None => write!(f, "{:?} error", self.kind),
        }
    }
}

impl std::error::Error for Error {}
