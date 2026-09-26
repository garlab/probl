//! Runtime errors.

use probl_syntax::{Diagnostic, Span};

/// An error from an operation that doesn't know where in the source it is;
/// the interpreter attaches the span.
#[derive(Clone, Debug, PartialEq)]
pub struct OpError {
    pub message: String,
    pub help: Option<String>,
    /// The program uses a feature that isn't implemented yet.
    pub unsupported: bool,
}

impl OpError {
    pub fn new(message: impl Into<String>) -> OpError {
        OpError {
            message: message.into(),
            help: None,
            unsupported: false,
        }
    }

    pub fn help(mut self, help: impl Into<String>) -> OpError {
        self.help = Some(help.into());
        self
    }

    pub fn unsupported(message: impl Into<String>) -> OpError {
        OpError {
            message: message.into(),
            help: None,
            unsupported: true,
        }
    }

    pub fn at(self, span: Span) -> RuntimeError {
        RuntimeError {
            message: self.message,
            span,
            notes: Vec::new(),
            help: self.help,
            unsupported: self.unsupported,
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
    pub unsupported: bool,
}

impl RuntimeError {
    pub fn new(span: Span, message: impl Into<String>) -> RuntimeError {
        OpError::new(message).at(span)
    }

    pub fn with_note(mut self, note: impl Into<String>) -> RuntimeError {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> RuntimeError {
        self.help = Some(help.into());
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
