//! What the command line shares with its tests: how it finds a program's
//! data (docs/data-input.md).

use probl_engine::data::Resolver;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Local files, for someone running their own programs: the command line
/// trusts the program, but not with anything but files.
///
/// - A relative path is relative to the directory of the program's file,
///   as it was named: a program reached through a link reads data next to
///   the link, not next to its target. An absolute path is used as it is,
///   and links in data paths are followed.
/// - Only regular files are read, so a program can't make the command wait
///   on a device or a pipe.
/// - `-` is standard input, where it's allowed (not in the REPL, where
///   standard input is the session). It's read on a thread of its own, so
///   that cancelling (`--timeout`) ends a wait for data that doesn't come.
pub struct LocalFiles {
    base: PathBuf,
    stdin: bool,
    cancel: Option<Arc<AtomicBool>>,
    /// Each identity given out, and the file it stands for.
    resolved: Vec<(String, PathBuf)>,
}

impl LocalFiles {
    /// Paths relative to `base`; `stdin` says whether `-` may be read.
    pub fn new(base: impl Into<PathBuf>, stdin: bool, cancel: Option<Arc<AtomicBool>>) -> LocalFiles {
        LocalFiles {
            base: base.into(),
            stdin,
            cancel,
            resolved: Vec::new(),
        }
    }

    /// For the program in the file `program`: relative to its directory.
    pub fn next_to(program: &Path, cancel: Option<Arc<AtomicBool>>) -> LocalFiles {
        let base = program.parent().map_or_else(PathBuf::new, Path::to_path_buf);
        LocalFiles::new(base, true, cancel)
    }
}

impl Resolver for LocalFiles {
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
    cancel: Option<Arc<AtomicBool>>,
    chunk: Vec<u8>,
    at: usize,
    done: bool,
}

impl Stdin {
    fn new(cancel: Option<Arc<AtomicBool>>) -> Stdin {
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
                    if self.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
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

/// A size on the command line: bytes, or with a unit like `64M` or `1G`.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let (number, unit) = match t.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => (&t[..i], t[i..].to_ascii_lowercase()),
        None => (t, String::new()),
    };
    let scale: u64 = match unit.trim_end_matches(['b', 'i']) {
        "" => 1,
        "k" => 1 << 10,
        "m" => 1 << 20,
        "g" => 1 << 30,
        _ => return Err(format!("can't read the size {text:?}: write it like 64M or 1G")),
    };
    let n: u64 = number
        .trim()
        .parse()
        .map_err(|_| format!("can't read the size {text:?}: write it like 64M or 1G"))?;
    n.checked_mul(scale)
        .ok_or_else(|| format!("the size {text:?} is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_have_units() {
        assert_eq!(parse_size("1024"), Ok(1024));
        assert_eq!(parse_size("64M"), Ok(64 << 20));
        assert_eq!(parse_size("64MiB"), Ok(64 << 20));
        assert_eq!(parse_size("2g"), Ok(2 << 30));
        assert_eq!(parse_size("500k"), Ok(500 << 10));
        assert!(parse_size("lots").is_err());
        assert!(parse_size("5T").is_err());
    }

    #[test]
    fn only_regular_files_are_read() {
        let dir = std::env::temp_dir();
        let mut files = LocalFiles::new(&dir, false, None);
        let id = files.resolve(".").unwrap();
        assert_eq!(files.open(&id).err().unwrap(), "it's a directory");
        let id = files.resolve("no-such-file-for-probl.csv").unwrap();
        assert_eq!(files.open(&id).err().unwrap(), "there's no such file");
        assert!(files.resolve("-").is_err());
    }
}
