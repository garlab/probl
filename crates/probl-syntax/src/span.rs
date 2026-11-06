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

/// The files of a program, in one space of offsets: each file's text starts
/// at its base, so a span says which file it's in. A program of one file has
/// it at 0, so its spans are its own offsets.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    /// Each file with its base, in increasing order.
    files: Vec<(u32, std::sync::Arc<SourceFile>)>,
    /// Every file's text at its base, a newline between two, so that the
    /// text of any span can be read directly.
    text: String,
}

impl SourceMap {
    pub fn new() -> SourceMap {
        SourceMap::default()
    }

    /// A map of one file, at 0.
    pub fn single(file: SourceFile) -> SourceMap {
        let mut map = SourceMap::new();
        map.add(file);
        map
    }

    /// Add a file after the others, and return its base.
    pub fn add(&mut self, file: SourceFile) -> u32 {
        if !self.files.is_empty() {
            self.text.push('\n');
        }
        let base = self.text.len() as u32;
        self.text.push_str(&file.text);
        self.files.push((base, std::sync::Arc::new(file)));
        base
    }

    /// Every file's text at its base: `&self.text()[span.range()]` is the
    /// text of `span`.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn files(&self) -> impl Iterator<Item = (u32, &std::sync::Arc<SourceFile>)> {
        self.files.iter().map(|(base, file)| (*base, file))
    }

    /// The position, among the files in the order they were added, of the
    /// file that `offset` is in.
    pub fn file_index(&self, offset: u32) -> Option<usize> {
        self.files.partition_point(|(base, _)| *base <= offset).checked_sub(1)
    }

    /// The file `span` starts in, and the span within that file.
    pub fn locate(&self, span: Span) -> Option<(&std::sync::Arc<SourceFile>, Span)> {
        let i = self
            .files
            .partition_point(|(base, _)| *base <= span.lo)
            .checked_sub(1)?;
        let (base, file) = &self.files[i];
        let end = base + file.text.len() as u32;
        Some((
            file,
            Span {
                lo: span.lo - base,
                hi: span.hi.min(end).max(span.lo) - base,
            },
        ))
    }
}
