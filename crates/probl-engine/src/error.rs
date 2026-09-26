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

/// An error from an operation that doesn't know where in the source it is;
/// the interpreter attaches the span.
#[derive(Clone, Debug, PartialEq)]
pub struct OpError {
    pub message: String,
    pub help: Option<String>,
    pub kind: ErrorKind,
}

impl OpError {
    pub fn new(message: impl Into<String>) -> OpError {
        OpError {
            message: message.into(),
            help: None,
            kind: ErrorKind::Language,
        }
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
        }
    }
}

pub type OpResult<T> = std::result::Result<T, OpError>;

#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeError {
    pub message: String,
    pub span: Span,
    pub notes: Vec<String>,
    pub help: Option<String>,
    pub kind: ErrorKind,
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

    pub fn is_unsupported(&self) -> bool {
        self.kind == ErrorKind::Unsupported
    }

    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut d = Diagnostic::error(self.span, &self.message);
        d.notes = self.notes.clone();
        d.help = self.help.clone();
        d
    }
}

pub type Result<T> = std::result::Result<T, RuntimeError>;
