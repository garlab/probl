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

/// A language error that depends on the values a world computes with, not
/// on the program being wrong: dividing by zero, an index past the end. In
/// partial mode it ends only the world it happens in (docs/semantics.md,
/// section 11). Errors without one always stop the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fault {
    /// Division or remainder by zero.
    DivisionByZero,
    /// A value outside what an operation is defined for: `sqrt(-1)`,
    /// `logit(0%)`, a distribution's parameter.
    Domain,
    /// An index past the end, or a key that isn't there.
    Index,
    /// A collection or a bag with nothing in it, where something is needed.
    Empty,
    /// An explicit conversion that can't represent the value.
    Conversion,
    /// A result too large to represent: a float that isn't finite, a date
    /// out of range.
    Overflow,
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
            help: self.help,
            kind: self.kind,
            fault: self.fault,
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
    pub help: Option<String>,
    pub kind: ErrorKind,
    pub fault: Option<Fault>,
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
        self.help = Some(help.into());
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
        d.help = self.help.clone();
        d
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
