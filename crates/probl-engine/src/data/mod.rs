//! Loading a program's data (docs/data-input.md).
//!
//! A program's manifest (`Program::inputs`) says what it reads. The host
//! opens each input through a [`Resolver`], which decides what a path means
//! and whether it may be read at all. This module reads the bytes, within
//! [`InputLimits`], and turns them into values of the declared types. The
//! engine never touches files: it gets the values as [`Inputs`], which only
//! [`load`] makes, and checks they're for the program it runs.

mod csv;
mod json;
mod schema;
mod text;

pub use schema::suggest;

use crate::error::{ErrorKind, RuntimeError};
use crate::report::thousands;
use crate::value::Value;
use probl_sema::ir::{DataFormat, Input, Program, TypeSpec};
use rustc_hash::FxHashMap;
use sha2::{Digest, Sha256};
use std::io::{ErrorKind as IoErrorKind, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// How a host gives a program its data. A program only names paths: what
/// they refer to, and whether they may be read, is the host's policy, and a
/// program can't widen it.
pub trait Resolver {
    /// What `path` refers to, as written in the program (`-` is standard
    /// input): an identity, such as a full file name. Two inputs with the
    /// same identity read the same bytes. Or why it may not be read.
    fn resolve(&mut self, path: &str) -> Result<String, String>;
    /// Open the data with this identity.
    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String>;
}

/// Limits on loading data, for all of a program's inputs together.
#[derive(Clone, Debug)]
pub struct InputLimits {
    /// Maximum magnitude size for each integer.
    pub max_integer_bits: u64,
    /// Cumulative allowance for large integer payloads across all inputs.
    pub max_integer_bytes: u64,
    /// Bytes read.
    pub max_bytes: u64,
    /// Values made: every number, string, record and element.
    pub max_values: u64,
    /// Elements of one list, map or bag.
    pub max_collection: usize,
    /// How deeply JSON arrays and objects may nest.
    pub max_depth: usize,
}

impl Default for InputLimits {
    fn default() -> InputLimits {
        InputLimits {
            max_integer_bits: probl_number::MAX_INTEGER_BITS,
            max_integer_bytes: 256 * 1024 * 1024,
            max_bytes: 64 * 1024 * 1024,
            max_values: 10_000_000,
            max_collection: 10_000_000,
            max_depth: 64,
        }
    }
}

/// The bytes read so far, by identity. A load reads each identity once;
/// kept between loads, snapshots keep the data the same. The REPL, which
/// runs its whole session again after each input, keeps them until
/// `:reload`, so that earlier bindings don't change under it.
#[derive(Debug, Default)]
pub struct Snapshots {
    bytes: FxHashMap<String, Arc<[u8]>>,
}

impl Snapshots {
    /// Forget everything read, so the next load reads it again.
    pub fn clear(&mut self) {
        self.bytes.clear();
    }
}

/// What was read for a program: the value of each input, and where they
/// came from. Only [`load`] makes these, and the engine checks that they're
/// for the program it runs.
#[derive(Debug)]
pub struct Inputs {
    max_integer_bits: u64,
    manifest: Vec<Input>,
    values: Vec<Value>,
    sources: Vec<SourceInfo>,
}

impl Inputs {
    pub(crate) fn max_integer_bits(&self) -> u64 {
        self.max_integer_bits
    }

    /// The value of each input, in the order of the program's manifest.
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Each file (or standard input) read, once.
    pub fn sources(&self) -> &[SourceInfo] {
        &self.sources
    }

    /// Whether these are the inputs of `program`.
    pub fn fit(&self, program: &Program) -> bool {
        self.manifest == program.inputs
    }
}

/// A file, or standard input, that was read.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceInfo {
    pub identity: String,
    pub bytes: u64,
    /// The SHA-256 of the bytes, in hex: what the results were computed
    /// from.
    pub sha256: String,
}

/// Read every input of `program`, through `resolver`. Errors are about the
/// input's `read(…)` call, and say where in the data they are.
pub fn load(
    program: &Program,
    resolver: &mut dyn Resolver,
    snapshots: &mut Snapshots,
    limits: &InputLimits,
    cancel: Option<&AtomicBool>,
) -> Result<Inputs, RuntimeError> {
    let mut cx = Cx {
        program,
        budget: Budget {
            values_left: limits.max_values,
            integer_bytes_left: limits.max_integer_bytes,
            max_integer_bits_seen: 0,
            limits,
            cancel,
            ticks: 0,
        },
        names: FxHashMap::default(),
        depth: 0,
        path: Vec::new(),
        problem: None,
    };
    // Each identity once, in the order they're first read.
    let mut read: Vec<(String, Arc<[u8]>)> = Vec::new();
    let mut total: u64 = 0;
    let mut values = Vec::with_capacity(program.inputs.len());
    for input in &program.inputs {
        let identity = resolver
            .resolve(&input.path)
            .map_err(|why| failure(input, Problem::new(format!("can't read it: {why}"))))?;
        let bytes = match read.iter().find(|(id, _)| *id == identity) {
            Some((_, bytes)) => bytes.clone(),
            None => {
                let bytes = match snapshots.bytes.get(&identity) {
                    Some(bytes) => bytes.clone(),
                    None => {
                        let reader = resolver
                            .open(&identity)
                            .map_err(|why| failure(input, Problem::new(format!("can't read it: {why}"))))?;
                        let left = limits.max_bytes.saturating_sub(total);
                        let bytes: Arc<[u8]> = read_bounded(reader, left, limits.max_bytes, cancel)
                            .map_err(|p| failure(input, p))?
                            .into();
                        snapshots.bytes.insert(identity.clone(), bytes.clone());
                        bytes
                    }
                };
                total += bytes.len() as u64;
                if total > limits.max_bytes {
                    return Err(failure(input, too_many_bytes(limits.max_bytes)));
                }
                read.push((identity, bytes.clone()));
                bytes
            }
        };
        let value = decode(&bytes, input, &mut cx).map_err(|p| failure(input, p))?;
        values.push(value);
    }
    let sources = read
        .iter()
        .map(|(identity, bytes)| SourceInfo {
            identity: identity.clone(),
            bytes: bytes.len() as u64,
            sha256: Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect(),
        })
        .collect();
    Ok(Inputs {
        max_integer_bits: cx.budget.max_integer_bits_seen,
        manifest: program.inputs.clone(),
        values,
        sources,
    })
}

/// Read all of `reader`, if it has at most `left` bytes, checking for
/// cancellation as it goes.
fn read_bounded(
    mut reader: Box<dyn Read + Send>,
    left: u64,
    max: u64,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<u8>, Problem> {
    let mut bytes = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(Problem::limit("reading it was cancelled"));
        }
        let n = match reader.read(&mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(n) => n,
            Err(e) if e.kind() == IoErrorKind::Interrupted => continue,
            Err(_) if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) => {
                return Err(Problem::limit("reading it was cancelled"));
            }
            Err(e) => return Err(Problem::new(format!("reading it failed: {e}"))),
        };
        if (bytes.len() + n) as u64 > left {
            return Err(too_many_bytes(max));
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
}

fn too_many_bytes(max: u64) -> Problem {
    Problem::limit(format!(
        "the data is more than {} MiB, the limit for all the data together",
        max.div_ceil(1024 * 1024)
    ))
    .help("raise the limit with `--max-input`")
}

/// Read one input's bytes as its declared type.
fn decode(bytes: &[u8], input: &Input, cx: &mut Cx) -> Result<Value, Problem> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    cx.depth = 0;
    cx.path.clear();
    cx.problem = None;
    match input.format {
        DataFormat::Csv => csv::read(bytes, &input.ty, cx),
        DataFormat::Json => json::read(bytes, &input.ty, cx),
        DataFormat::Lines => lines(bytes, &input.ty, cx),
    }
}

/// One value per non-blank line.
fn lines(bytes: &[u8], ty: &TypeSpec, cx: &mut Cx) -> Result<Value, Problem> {
    let TypeSpec::List(item) = ty else {
        return Err(Problem::new("lines read as a list"));
    };
    let text = std::str::from_utf8(bytes).map_err(|e| {
        let line = bytes[..e.valid_up_to()].iter().filter(|&&b| b == b'\n').count() + 1;
        Problem::new("isn't text: it isn't valid UTF-8").at(format!("line {line}"))
    })?;
    let mut items = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        cx.budget.collection(items.len() + 1)?;
        cx.budget.value()?;
        let v = text::plain(line, item, cx.program, &mut cx.budget).map_err(|p| p.at(format!("line {}", i + 1)))?;
        items.push(v);
    }
    Ok(Value::list(items))
}

/// What decoding needs: the program's types, the budget, and where it is.
pub(crate) struct Cx<'a> {
    program: &'a Program,
    budget: Budget<'a>,
    /// Record type and field names, made once.
    names: FxHashMap<String, Arc<str>>,
    /// JSON nesting.
    depth: usize,
    /// Where in a JSON document: keys and indices.
    path: Vec<PathPart>,
    /// The first problem found inside the JSON parser, which only passes on
    /// its own errors.
    problem: Option<Problem>,
}

impl Cx<'_> {
    fn name(&mut self, name: &str) -> Arc<str> {
        if let Some(n) = self.names.get(name) {
            return n.clone();
        }
        let n: Arc<str> = Arc::from(name);
        self.names.insert(name.to_string(), n.clone());
        n
    }
}

enum PathPart {
    Key(String),
    Index(usize),
}

/// Limits being spent while decoding.
struct Budget<'a> {
    values_left: u64,
    integer_bytes_left: u64,
    max_integer_bits_seen: u64,
    limits: &'a InputLimits,
    cancel: Option<&'a AtomicBool>,
    ticks: u32,
}

impl Budget<'_> {
    fn integer(&mut self, bits: u64) -> Result<(), Problem> {
        let limit = self.limits.max_integer_bits.min(probl_number::MAX_INTEGER_BITS);
        if bits > limit {
            return Err(Problem::limit(format!(
                "integer size exceeds the limit of {limit} bits"
            )));
        }
        self.max_integer_bits_seen = self.max_integer_bits_seen.max(bits);
        if bits > 63 {
            let bytes = bits.div_ceil(64) * 8 + 48;
            self.integer_bytes_left = self
                .integer_bytes_left
                .checked_sub(bytes)
                .ok_or_else(|| Problem::limit("the data used up its large integer memory allowance"))?;
        }
        Ok(())
    }

    /// Count one more value, before making it.
    fn value(&mut self) -> Result<(), Problem> {
        if self.values_left == 0 {
            return Err(Problem::limit(format!(
                "the data has more than {} values, the limit for all the data together",
                thousands(self.limits.max_values.min(i64::MAX as u64) as i64)
            )));
        }
        self.values_left -= 1;
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks % 4096 == 0 && self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(Problem::limit("reading it was cancelled"));
        }
        Ok(())
    }

    /// Check that a collection may have `n` elements.
    fn collection(&self, n: usize) -> Result<(), Problem> {
        if n > self.limits.max_collection {
            return Err(Problem::limit(format!(
                "a list, map or bag has more than {} elements",
                thousands(self.limits.max_collection as i64)
            )));
        }
        Ok(())
    }
}

/// What's wrong with the data, and where.
#[derive(Clone, Debug)]
pub(crate) struct Problem {
    message: String,
    help: Option<String>,
    notes: Vec<String>,
    /// Where in the data: "line 12, column `signups`".
    at: Option<String>,
    limit: bool,
}

impl Problem {
    fn new(message: impl Into<String>) -> Problem {
        Problem {
            message: message.into(),
            help: None,
            notes: Vec::new(),
            at: None,
            limit: false,
        }
    }

    fn limit(message: impl Into<String>) -> Problem {
        Problem {
            limit: true,
            ..Problem::new(message)
        }
    }

    fn help(mut self, help: impl Into<String>) -> Problem {
        self.help = Some(help.into());
        self
    }

    fn note(mut self, note: impl Into<String>) -> Problem {
        self.notes.push(note.into());
        self
    }

    /// Say where, unless it's already said.
    fn at(mut self, at: impl Into<String>) -> Problem {
        if self.at.is_none() {
            self.at = Some(at.into());
        }
        self
    }
}

/// A problem with an input, as an error at its `read(…)` call.
fn failure(input: &Input, problem: Problem) -> RuntimeError {
    let place = match &problem.at {
        Some(at) => format!("{}, {at}", source_name(input)),
        None => source_name(input),
    };
    let mut error = RuntimeError::new(input.span, format!("{place}: {}", problem.message));
    error.notes = problem.notes;
    error.help = problem.help;
    if problem.limit {
        error.kind = ErrorKind::Limit;
    }
    error
}

fn source_name(input: &Input) -> String {
    match input.path.as_str() {
        "-" => "standard input".to_string(),
        path => path.to_string(),
    }
}

/// A short, quoted rendering of text from the data, for messages.
fn quoted(text: &str) -> String {
    const MAX: usize = 40;
    match text.char_indices().nth(MAX) {
        Some((cut, _)) => format!("`{}…`", &text[..cut]),
        None => format!("`{text}`"),
    }
}

#[cfg(test)]
mod tests;
