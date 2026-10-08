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

/// A problem compiling a program, loading its data or running it.
#[derive(Clone, Debug)]
pub struct Error {
    kind: ErrorKind,
    diagnostics: Vec<Diagnostic>,
    partial: Option<Arc<Outcome>>,
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
        }
    }

    pub(crate) fn compile(diagnostics: Vec<probl_syntax::Diagnostic>, file: &Arc<SourceFile>) -> Error {
        Error {
            kind: ErrorKind::Compile,
            diagnostics: diagnostics.into_iter().map(|d| Diagnostic::new(d, file)).collect(),
            partial: None,
        }
    }

    pub(crate) fn runtime(e: RuntimeError, file: &Arc<SourceFile>) -> Error {
        let kind = match e.kind {
            probl_engine::ErrorKind::Language => ErrorKind::Language,
            probl_engine::ErrorKind::Unsupported => ErrorKind::Unsupported,
            probl_engine::ErrorKind::Limit => ErrorKind::Limit,
            probl_engine::ErrorKind::Internal => ErrorKind::Internal,
        };
        Error {
            kind,
            diagnostics: vec![Diagnostic::new(e.to_diagnostic(), file)],
            partial: None,
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
