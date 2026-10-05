//! Where a program's data comes from (docs/data-input.md): what `read("…")`
//! finds, and the data once it's loaded.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, PoisonError};

/// Where `read("…")` finds data. A program only names paths: what they refer
/// to, and whether they may be read at all, is the host's to decide, and a
/// program can't widen it.
pub trait Files {
    /// What `path`, as the program writes it, refers to: an identity, such as
    /// a full file name, or why it may not be read. `-` is standard input.
    /// Two paths with the same identity read the same data.
    fn resolve(&mut self, path: &str) -> Result<String, String>;

    /// Read what an identity stands for.
    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String>;
}

/// The engine's resolver, for any [`Files`].
pub(crate) struct Resolve<'a>(pub &'a mut dyn Files);

impl probl_engine::data::Resolver for Resolve<'_> {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        self.0.resolve(path)
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        self.0.open(identity)
    }
}

/// Files given as bytes, by the path a program reads them with. There's no
/// standard input.
#[derive(Clone, Debug, Default)]
pub struct MemoryFiles {
    files: BTreeMap<String, Arc<[u8]>>,
}

impl MemoryFiles {
    pub fn new() -> MemoryFiles {
        MemoryFiles::default()
    }

    /// Add a file, or replace one.
    pub fn insert(&mut self, path: impl Into<String>, bytes: impl Into<Vec<u8>>) {
        self.files.insert(path.into(), bytes.into().into());
    }

    pub fn contains(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
}

impl<P: Into<String>, B: Into<Vec<u8>>> FromIterator<(P, B)> for MemoryFiles {
    fn from_iter<I: IntoIterator<Item = (P, B)>>(files: I) -> MemoryFiles {
        let mut memory = MemoryFiles::new();
        for (path, bytes) in files {
            memory.insert(path, bytes);
        }
        memory
    }
}

impl Files for MemoryFiles {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        if path == "-" {
            return Err("there's no standard input".to_string());
        }
        if self.files.contains_key(path) {
            Ok(path.to_string())
        } else {
            Err(format!("`{path}` isn't one of the files given"))
        }
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        let bytes = self.files.get(identity).ok_or("it wasn't resolved")?.clone();
        Ok(Box::new(io::Cursor::new(bytes)))
    }
}

/// Another [`Files`], keeping the bytes of everything it has read, so that
/// reading it again gives the same data until [`clear`](Snapshots::clear).
/// The REPL keeps its data this way: it runs a session whose program changes
/// with every input, and earlier bindings mustn't change under it.
pub struct Snapshots<F> {
    files: F,
    kept: Arc<Mutex<BTreeMap<String, Arc<[u8]>>>>,
}

impl<F: Files> Snapshots<F> {
    pub fn new(files: F) -> Snapshots<F> {
        Snapshots {
            files,
            kept: Arc::default(),
        }
    }

    /// Forget everything read, so that it's read again.
    pub fn clear(&mut self) {
        self.kept.lock().unwrap_or_else(PoisonError::into_inner).clear();
    }

    pub fn get_ref(&self) -> &F {
        &self.files
    }

    pub fn get_mut(&mut self) -> &mut F {
        &mut self.files
    }
}

impl<F: Files> Files for Snapshots<F> {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        self.files.resolve(path)
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        let kept = self
            .kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(identity)
            .cloned();
        if let Some(bytes) = kept {
            return Ok(Box::new(io::Cursor::new(bytes)));
        }
        Ok(Box::new(Keeping {
            reader: self.files.open(identity)?,
            bytes: Vec::new(),
            identity: identity.to_string(),
            kept: self.kept.clone(),
        }))
    }
}

/// A reader that keeps what it reads, once it has read all of it: a read
/// stopped by a limit keeps nothing.
struct Keeping {
    reader: Box<dyn Read + Send>,
    bytes: Vec<u8>,
    identity: String,
    kept: Arc<Mutex<BTreeMap<String, Arc<[u8]>>>>,
}

impl Read for Keeping {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        if n == 0 {
            let bytes = std::mem::take(&mut self.bytes);
            let mut kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            kept.entry(std::mem::take(&mut self.identity))
                .or_insert_with(|| bytes.into());
        } else {
            self.bytes.extend_from_slice(&buf[..n]);
        }
        Ok(n)
    }
}

/// A program's data, read by [`Program::load`](crate::Program::load). It
/// can be given to any number of runs of that program.
#[derive(Clone)]
pub struct Data {
    pub(crate) inputs: Arc<probl_engine::data::Inputs>,
    sources: Vec<DataSource>,
}

impl Data {
    pub(crate) fn new(inputs: probl_engine::data::Inputs) -> Data {
        let sources = inputs.sources().iter().map(DataSource::new).collect();
        Data {
            inputs: Arc::new(inputs),
            sources,
        }
    }

    /// Each file (or standard input) read, once.
    pub fn sources(&self) -> &[DataSource] {
        &self.sources
    }
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data").field("sources", &self.sources).finish()
    }
}

/// A file, or standard input, that was read: what results were computed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataSource {
    identity: String,
    bytes: u64,
    sha256: String,
}

impl DataSource {
    pub(crate) fn new(source: &probl_engine::data::SourceInfo) -> DataSource {
        DataSource {
            identity: source.identity.clone(),
            bytes: source.bytes,
            sha256: source.sha256.clone(),
        }
    }

    /// What its path resolved to.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Its size in bytes.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// The SHA-256 of its bytes, in hex.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use local::LocalFiles;

#[cfg(not(target_arch = "wasm32"))]
mod local {
    use super::Files;
    use crate::Cancel;
    use std::io::{self, Read};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::Duration;

    /// Local files, for someone running their own programs: the program is
    /// trusted, but not with anything but files.
    ///
    /// - A relative path is relative to a base directory: for
    ///   [`next_to`](LocalFiles::next_to), the directory of the program's
    ///   file, as it was named, so that a program reached through a link reads
    ///   data next to the link, not next to its target. An absolute path is
    ///   used as it is, and links in data paths are followed.
    /// - Only regular files are read, so that a program can't make its host
    ///   wait on a device or a pipe.
    /// - `-` is standard input, when [`stdin`](LocalFiles::stdin) allows it.
    ///   It's read on a thread of its own, so that cancelling ends a wait for
    ///   data that doesn't come.
    pub struct LocalFiles {
        base: PathBuf,
        stdin: bool,
        cancel: Option<Cancel>,
        /// Each identity given out, and the file it stands for.
        resolved: Vec<(String, PathBuf)>,
    }

    impl LocalFiles {
        /// Paths relative to `base`, without standard input.
        pub fn new(base: impl Into<PathBuf>) -> LocalFiles {
            LocalFiles {
                base: base.into(),
                stdin: false,
                cancel: None,
                resolved: Vec::new(),
            }
        }

        /// For the program in the file `program`: paths relative to its
        /// directory, without standard input.
        pub fn next_to(program: &Path) -> LocalFiles {
            LocalFiles::new(program.parent().map_or_else(PathBuf::new, Path::to_path_buf))
        }

        /// Whether `-` may be read, as standard input.
        pub fn stdin(mut self, allowed: bool) -> LocalFiles {
            self.stdin = allowed;
            self
        }

        /// Stop waiting for standard input when `cancel` is cancelled.
        pub fn cancel(mut self, cancel: &Cancel) -> LocalFiles {
            self.cancel = Some(cancel.clone());
            self
        }
    }

    impl Files for LocalFiles {
        fn resolve(&mut self, path: &str) -> Result<String, String> {
            if path == "-" {
                return if self.stdin {
                    Ok("-".to_string())
                } else {
                    Err("standard input can't be read here".to_string())
                };
            }
            let full = self.base.join(path);
            let identity = full.display().to_string();
            if !self.resolved.iter().any(|(id, _)| *id == identity) {
                self.resolved.push((identity.clone(), full));
            }
            Ok(identity)
        }

        fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
            if identity == "-" {
                return Ok(Box::new(Stdin::new(self.cancel.clone())));
            }
            let path = self
                .resolved
                .iter()
                .find(|(id, _)| id == identity)
                .map(|(_, p)| p.clone())
                .ok_or("it wasn't resolved")?;
            // Before opening: opening a pipe would wait for a writer.
            let meta = std::fs::metadata(&path).map_err(describe)?;
            if meta.is_dir() {
                return Err("it's a directory".to_string());
            }
            if !meta.is_file() {
                return Err("it isn't a regular file".to_string());
            }
            let file = std::fs::File::open(&path).map_err(describe)?;
            Ok(Box::new(file))
        }
    }

    fn describe(e: io::Error) -> String {
        match e.kind() {
            io::ErrorKind::NotFound => "there's no such file".to_string(),
            io::ErrorKind::PermissionDenied => "permission denied".to_string(),
            _ => e.to_string(),
        }
    }

    /// Standard input, read on a thread of its own, in chunks.
    struct Stdin {
        chunks: mpsc::Receiver<io::Result<Vec<u8>>>,
        cancel: Option<Cancel>,
        chunk: Vec<u8>,
        at: usize,
        done: bool,
    }

    impl Stdin {
        fn new(cancel: Option<Cancel>) -> Stdin {
            let (tx, chunks) = mpsc::channel();
            std::thread::spawn(move || {
                let mut stdin = io::stdin().lock();
                loop {
                    let mut chunk = vec![0; 64 * 1024];
                    match stdin.read(&mut chunk) {
                        Ok(n) => {
                            chunk.truncate(n);
                            if tx.send(Ok(chunk)).is_err() || n == 0 {
                                return;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            return;
                        }
                    }
                }
            });
            Stdin {
                chunks,
                cancel,
                chunk: Vec::new(),
                at: 0,
                done: false,
            }
        }
    }

    impl Read for Stdin {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            while self.at == self.chunk.len() {
                if self.done {
                    return Ok(0);
                }
                match self.chunks.recv_timeout(Duration::from_millis(50)) {
                    Ok(Ok(chunk)) => {
                        self.done = chunk.is_empty();
                        self.chunk = chunk;
                        self.at = 0;
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if self.cancel.as_ref().is_some_and(Cancel::is_cancelled) {
                            return Err(io::Error::other("waiting for it was cancelled"));
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => self.done = true,
                }
            }
            let n = buf.len().min(self.chunk.len() - self.at);
            buf[..n].copy_from_slice(&self.chunk[self.at..self.at + n]);
            self.at += n;
            Ok(n)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_regular_files_are_read() {
            let dir = std::env::temp_dir();
            let mut files = LocalFiles::new(&dir);
            let id = files.resolve(".").unwrap();
            assert_eq!(files.open(&id).err().unwrap(), "it's a directory");
            let id = files.resolve("no-such-file-for-probl.csv").unwrap();
            assert_eq!(files.open(&id).err().unwrap(), "there's no such file");
            assert!(files.resolve("-").is_err());
            assert!(LocalFiles::new(&dir).stdin(true).resolve("-").is_ok());
        }
    }
}
