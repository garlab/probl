//! Source text and byte-offset spans into it.

use std::fmt;

/// A half-open byte range `lo..hi` in a source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(lo: usize, hi: usize) -> Span {
        Span {
            lo: lo as u32,
            hi: hi as u32,
        }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn range(self) -> std::ops::Range<usize> {
        self.lo as usize..self.hi as usize
    }

    pub fn shifted(self, by: u32) -> Span {
        Span {
            lo: self.lo + by,
            hi: self.hi + by,
        }
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.lo, self.hi)
    }
}

/// A named piece of source text.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> SourceFile {
        SourceFile {
            name: name.into(),
            text: text.into(),
        }
    }

    /// The source text covered by `span`.
    pub fn slice(&self, span: Span) -> &str {
        self.text.get(span.range()).unwrap_or("")
    }

    /// 1-based line and column of a byte offset.
    pub fn line_col(&self, offset: u32) -> (usize, usize) {
        let offset = (offset as usize).min(self.text.len());
        let before = &self.text[..offset];
        let line = before.matches('\n').count() + 1;
        let col = before.rfind('\n').map_or(offset, |nl| offset - nl - 1) + 1;
        (line, col)
    }
}
