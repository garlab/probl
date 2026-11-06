//! Where the files a program imports come from
//! (docs/design/imports-and-tests.md).

use std::collections::BTreeMap;

/// Where the files a program imports come from. A program only names paths:
/// what they refer to, and whether they may be read at all, is the host's to
/// decide.
pub trait Sources {
    /// The file `path` names, imported by the file whose identity is
    /// `importer`. Two paths to the same file must give the same identity:
    /// a file runs once per run, however many files import it.
    fn load(&mut self, importer: &str, path: &str) -> Result<SourceText, String>;

    /// The identity of the entry file, compiled under `name`, which its
    /// imports are resolved against. By default, the name itself.
    fn entry(&mut self, name: &str) -> String {
        name.to_string()
    }
}

/// An imported file's text, with its identity and the name diagnostics give
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceText {
    identity: String,
    name: String,
    text: String,
}

impl SourceText {
    pub fn new(identity: impl Into<String>, name: impl Into<String>, text: impl Into<String>) -> SourceText {
        SourceText {
            identity: identity.into(),
            name: name.into(),
            text: text.into(),
        }
    }
}

/// The compiler's loader, for any [`Sources`].
pub(crate) struct Load<'a>(pub &'a mut dyn Sources);

impl probl_sema::Loader for Load<'_> {
    fn load(&mut self, importer: &str, path: &str) -> Result<probl_sema::Source, String> {
        let s = self.0.load(importer, path)?;
        Ok(probl_sema::Source {
            identity: s.identity,
            name: s.name,
            text: s.text,
        })
    }
}

/// `path`, relative to the directory of `importer`, with `.` and `..` worked
/// out and `/` between parts. `None` when it goes above the top.
fn join(importer: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = importer.split('/').collect();
    parts.pop();
    parts.retain(|p| !p.is_empty() && *p != ".");
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// Files given as text, by their paths, with `/` between directories. A file
/// imports another relative to its own directory, as on disk.
#[derive(Clone, Debug, Default)]
pub struct MemorySources {
    files: BTreeMap<String, String>,
}

impl MemorySources {
    pub fn new() -> MemorySources {
        MemorySources::default()
    }

    /// Add a file, or replace one.
    pub fn insert(&mut self, path: impl Into<String>, text: impl Into<String>) {
        self.files.insert(path.into(), text.into());
    }
}

impl<P: Into<String>, T: Into<String>> FromIterator<(P, T)> for MemorySources {
    fn from_iter<I: IntoIterator<Item = (P, T)>>(files: I) -> MemorySources {
        let mut memory = MemorySources::new();
        for (path, text) in files {
            memory.insert(path, text);
        }
        memory
    }
}

impl Sources for MemorySources {
    fn load(&mut self, importer: &str, path: &str) -> Result<SourceText, String> {
        let at = join(importer, path).ok_or("it's outside the files given")?;
        let text = self.files.get(&at).ok_or("there's no such file")?;
        Ok(SourceText::new(at.clone(), at, text.clone()))
    }

    fn entry(&mut self, name: &str) -> String {
        join("", name).unwrap_or_else(|| name.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub use local::LocalSources;

#[cfg(not(target_arch = "wasm32"))]
mod local {
    use super::{SourceText, Sources};
    use std::io::ErrorKind;
    use std::path::{Path, PathBuf};

    /// Files on disk: a file imports another relative to its own directory.
    /// A path must end in `.probl`. A file's identity is its canonical path,
    /// so a file reached through a link or by another relative path is the
    /// same file. Like [`LocalFiles`](crate::LocalFiles), this follows the
    /// local file system's rules; it isn't a sandbox.
    #[derive(Clone, Debug, Default)]
    pub struct LocalSources;

    impl LocalSources {
        pub fn new() -> LocalSources {
            LocalSources
        }
    }

    /// How to show `path` in diagnostics: relative to the current
    /// directory, when it's inside it.
    fn shown(path: &Path) -> String {
        let here = std::env::current_dir().and_then(std::fs::canonicalize).ok();
        match here.as_deref().and_then(|here| path.strip_prefix(here).ok()) {
            Some(relative) => relative.display().to_string(),
            None => path.display().to_string(),
        }
    }

    impl Sources for LocalSources {
        fn load(&mut self, importer: &str, path: &str) -> Result<SourceText, String> {
            if !path.ends_with(".probl") {
                return Err("an imported file's path ends in `.probl`".into());
            }
            let dir = Path::new(importer).parent().unwrap_or(Path::new(""));
            let joined: PathBuf = dir.join(path);
            let canonical = std::fs::canonicalize(&joined).map_err(|e| match e.kind() {
                ErrorKind::NotFound => "there's no such file".to_string(),
                _ => e.to_string(),
            })?;
            let meta = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
            if !meta.is_file() {
                return Err("it isn't a file".into());
            }
            if meta.len() > probl_sema::modules::MAX_SOURCE as u64 {
                return Err(format!("it's larger than {} bytes", probl_sema::modules::MAX_SOURCE));
            }
            let text = std::fs::read_to_string(&canonical).map_err(|e| match e.kind() {
                ErrorKind::InvalidData => "it isn't UTF-8 text".to_string(),
                _ => e.to_string(),
            })?;
            Ok(SourceText::new(
                canonical.display().to_string(),
                shown(&canonical),
                text,
            ))
        }

        fn entry(&mut self, name: &str) -> String {
            std::fs::canonicalize(name).map_or_else(|_| name.to_string(), |p| p.display().to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_relative_to_the_importing_file() {
        assert_eq!(join("main.probl", "./lib.probl").as_deref(), Some("lib.probl"));
        assert_eq!(
            join("a/b/main.probl", "../lib/x.probl").as_deref(),
            Some("a/lib/x.probl")
        );
        assert_eq!(join("main.probl", "../x.probl"), None);
        let mut files: MemorySources = [("main.probl", "1"), ("lib/a.probl", "2")].into_iter().collect();
        let a = files.load("main.probl", "./lib/a.probl").unwrap();
        assert_eq!(a, SourceText::new("lib/a.probl", "lib/a.probl", "2"));
        assert_eq!(
            files.load("lib/a.probl", "../main.probl").unwrap().identity,
            "main.probl"
        );
        assert_eq!(
            files.load("main.probl", "./b.probl"),
            Err("there's no such file".into())
        );
    }
}
