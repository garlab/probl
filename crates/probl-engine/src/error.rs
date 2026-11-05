//! Runtime errors.

use probl_syntax::{Diagnostic, Span};

/// What kind of problem stopped a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The program did something the language doesn't allow.
    Language,
    /// The program uses a feature that isn't implemented yet.
    Unsupported,
    /// The run hit a resource limit, or was cancelled.
    Limit,
    /// A bug in Probl.
    Internal,
}

pub use probl_sema::ir::Fault;

/// What a fault that stops a run can say about itself (stage 0 of
/// docs/design/error-handling.md). A part that's absent is unavailable, not
/// zero.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FaultContext {
    /// The failing operation's inputs other than literals, as the failing
    /// world or run had them.
    pub inputs: Vec<FaultInput>,
    /// When enumerating: of the weight reaching the operation, the share
    /// for which it faults.
    pub share: Option<f64>,
    /// `share` counts only the current pass through a loop.
    pub per_pass: bool,
    /// When sampling: the run that failed.
    pub run: Option<FailedRun>,
}

/// One input of a failing operation.
#[derive(Clone, Debug, PartialEq)]
pub struct FaultInput {
    /// Where it is in the source.
    pub span: Span,
    /// The variable's name, when the input is one.
    pub name: Option<String>,
    /// Its value, as text, cut short when long.
    pub value: String,
}

/// The sampled run in which a fault stopped the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FailedRun {
    /// From 0.
    pub run: u64,
    pub runs: u64,
    pub seed: u64,
}

/// An error from an operation that doesn't know where in the source it is;
/// the interpreter attaches the span.
#[derive(Clone, Debug, PartialEq)]
pub struct OpError {
    pub message: String,
    pub help: Option<String>,
    pub kind: ErrorKind,
    pub fault: Option<Fault>,
}

impl OpError {
    pub fn new(message: impl Into<String>) -> OpError {
        OpError {
            message: message.into(),
            help: None,
            kind: ErrorKind::Language,
            fault: None,
        }
    }

    /// A language error that ends only its world in partial mode.
    pub fn fault(fault: Fault, message: impl Into<String>) -> OpError {
        OpError {
            fault: Some(fault),
            ..OpError::new(message)
        }
    }

    /// The same error, as a fault.
    pub fn as_fault(mut self, fault: Fault) -> OpError {
        if self.kind == ErrorKind::Language {
            self.fault = Some(fault);
        }
        self
    }

    pub fn unsupported(message: impl Into<String>) -> OpError {
        OpError {
            kind: ErrorKind::Unsupported,
            ..OpError::new(message)
        }
    }

    pub fn limit(message: impl Into<String>) -> OpError {
        OpError {
            kind: ErrorKind::Limit,
            ..OpError::new(message)
        }
    }

    /// A bug in Probl: all the user can do is report it.
    pub fn internal(message: impl Into<String>) -> OpError {
        OpError {
            kind: ErrorKind::Internal,
            ..OpError::new(message)
        }
        .help("this is a bug in Probl; please report it with the program that caused it")
    }

    pub fn help(mut self, help: impl Into<String>) -> OpError {
        self.help = Some(help.into());
        self
    }

    pub fn at(self, span: Span) -> RuntimeError {
        RuntimeError {
            message: self.message,
            span,
            notes: Vec::new(),
            help: self.help.map(String::into_boxed_str),
            kind: self.kind,
            fault: self.fault,
            context: None,
        }
    }
}

pub type OpResult<T> = std::result::Result<T, OpError>;

impl From<probl_number::IntError> for OpError {
    fn from(error: probl_number::IntError) -> Self {
        match error {
            probl_number::IntError::TooLarge => Self::limit(error.to_string()),
            probl_number::IntError::DivisionByZero => Self::fault(Fault::DivisionByZero, error.to_string()),
            _ => Self::new(error.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
    pub notes: Vec<String>,
    /// Boxed: with `context`, it keeps the error, and every `Result` carrying
    /// it, as small as it was (88 bytes).
    pub help: Option<Box<str>>,
    pub kind: ErrorKind,
    pub fault: Option<Fault>,
    /// For a fault: its operation's inputs and, once it stopped the run,
    /// how often it fails. Hosts render it with the source.
    pub context: Option<Box<FaultContext>>,
}

impl RuntimeError {
    pub fn new(span: Span, message: impl Into<String>) -> RuntimeError {
        OpError::new(message).at(span)
    }

    pub fn limit(span: Span, message: impl Into<String>) -> RuntimeError {
        OpError::limit(message).at(span)
    }

    pub fn with_note(mut self, note: impl Into<String>) -> RuntimeError {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> RuntimeError {
        self.help = Some(help.into().into_boxed_str());
        self
    }

    /// The same error, as a fault.
    pub fn as_fault(mut self, fault: Fault) -> RuntimeError {
        if self.kind == ErrorKind::Language {
            self.fault = Some(fault);
        }
        self
    }

    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut d = Diagnostic::error(self.span, &self.message);
        d.notes = self.notes.clone();
        d.help = self.help.as_deref().map(str::to_string);
        d
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
