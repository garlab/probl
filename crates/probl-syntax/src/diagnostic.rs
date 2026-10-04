//! Errors and warnings with source locations, rendered with `ariadne`.

use crate::span::{SourceFile, Span};
use ariadne::{Config, IndexType, Label, Report, ReportKind, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    /// Short text shown under the highlighted span.
    pub label: Option<String>,
    pub notes: Vec<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span,
            label: None,
            notes: Vec::new(),
            help: None,
        }
    }

    pub fn warning(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            ..Diagnostic::error(span, message)
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Diagnostic {
        self.label = Some(label.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Diagnostic {
        self.help = Some(help.into());
        self
    }

    /// Add help text to a diagnostic that has already been recorded.
    pub fn help(&mut self, help: impl Into<String>) -> &mut Diagnostic {
        self.help = Some(help.into());
        self
    }

    /// Add a note to a diagnostic that has already been recorded.
    pub fn note(&mut self, note: impl Into<String>) -> &mut Diagnostic {
        self.notes.push(note.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Render the diagnostic with a source snippet.
    pub fn render(&self, file: &SourceFile, color: bool) -> String {
        let name = file.name.as_str();
        let kind = match self.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let range = clamp(self.span, file.text.len());
        // ariadne only draws the marker under a span when its label has text.
        let label = Label::new((name, range.clone())).with_message(self.label.as_deref().unwrap_or("here"));
        let mut report = Report::build(kind, (name, range))
            .with_config(Config::default().with_color(color).with_index_type(IndexType::Byte))
            .with_message(&self.message)
            .with_label(label);
        for note in &self.notes {
            report = report.with_note(note);
        }
        if let Some(help) = &self.help {
            report = report.with_help(help);
        }
        let mut out = Vec::new();
        report
            .finish()
            .write((name, Source::from(file.text.as_str())), &mut out)
            .expect("writing to a Vec cannot fail");
        String::from_utf8_lossy(&out).into_owned()
    }
}

pub fn render_all(diagnostics: &[Diagnostic], file: &SourceFile, color: bool) -> String {
    diagnostics.iter().map(|d| d.render(file, color)).collect()
}

fn clamp(span: Span, len: usize) -> std::ops::Range<usize> {
    let lo = (span.lo as usize).min(len);
    let hi = (span.hi as usize).clamp(lo, len);
    lo..hi
}
