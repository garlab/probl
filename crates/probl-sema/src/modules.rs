//! Loading a program's files: its entry and every file it imports, each once,
//! in evaluation order (docs/design/imports-and-tests.md).

use crate::ir;
use crate::lower::{ModuleAst, lower_modules};
use crate::symbols::Symbols;
use probl_syntax::ast::{self, Item};
use probl_syntax::{Diagnostic, SourceFile, SourceMap, Span, parse_program_at};
use rustc_hash::FxHashMap;

/// The largest source text one file may have, in bytes.
pub const MAX_SOURCE: usize = 8 * 1024 * 1024;
/// The largest source text a program's files may have together, in bytes.
pub const MAX_TOTAL_SOURCE: usize = 32 * 1024 * 1024;
/// The most files a program may have.
pub const MAX_MODULES: usize = 1000;
/// The longest chain of imports, from the entry.
pub const MAX_DEPTH: usize = 64;

/// A program's file: the identity its imports are resolved against, the name
/// its diagnostics give it, and its text.
#[derive(Clone, Debug)]
pub struct Source {
    pub identity: String,
    pub name: String,
    pub text: String,
}

/// Where imported files come from. The host decides: from disk, relative to
/// the importing file, or from memory.
pub trait Loader {
    /// The file `path` names, imported by the file whose identity is
    /// `importer`. The same file must have the same identity however it's
    /// named, since a file is evaluated once per run.
    fn load(&mut self, importer: &str, path: &str) -> Result<Source, String>;

    /// The path a `read(path)` in the file `importer` names, for the host
    /// that loads the program's data, which reads paths relative to the
    /// entry file. By default, `path` relative to the directory of
    /// `importer`, with `/` between parts.
    fn data_path(&mut self, importer: &str, path: &str) -> String {
        if path.starts_with('/') {
            return path.to_string();
        }
        match importer.rsplit_once('/') {
            Some((dir, _)) => format!("{dir}/{}", path.strip_prefix("./").unwrap_or(path)),
            None => path.to_string(),
        }
    }
}

/// The loader of a program compiled on its own, which can't import.
pub struct NoImports;

impl Loader for NoImports {
    fn load(&mut self, _importer: &str, _path: &str) -> Result<Source, String> {
        Err("this program is compiled on its own, without its files; run it from its file, with `probl run`".into())
    }
}

/// A compiled program, with the files it's made of.
pub struct Compiled {
    /// Only when there are no errors.
    pub program: Option<ir::Program>,
    pub diagnostics: Vec<Diagnostic>,
    /// The files, whose offsets the program's and the diagnostics' spans are.
    pub sources: SourceMap,
    /// Where names are declared and used, when every file parses.
    pub symbols: Option<Symbols>,
}

/// Compile the program whose entry is `entry`, loading the files it imports
/// with `loader`.
pub fn compile_program(entry: Source, loader: &mut dyn Loader) -> Compiled {
    compile(entry, loader, true)
}

/// Compile a file as it's compiled when another file imports it, with the
/// files it imports: to check it on its own, as an editor does. It can't
/// `report` or set pragmas, and its program is only for checking.
pub fn compile_module(source: Source, loader: &mut dyn Loader) -> Compiled {
    compile(source, loader, false)
}

/// Compile the file `root` and the files it imports; `root` is the entry
/// when `entry` is true.
fn compile(root: Source, loader: &mut dyn Loader, entry: bool) -> Compiled {
    let mut graph = Graph {
        loader: &mut *loader,
        nodes: Vec::new(),
        by_identity: FxHashMap::default(),
        sources: SourceMap::new(),
        diags: Vec::new(),
        stack: Vec::new(),
        order: Vec::new(),
    };
    if root.text.len() > MAX_SOURCE {
        let d = Diagnostic::error(
            Span::default(),
            format!("the program is larger than {MAX_SOURCE} bytes"),
        );
        return Compiled {
            program: None,
            diagnostics: vec![d],
            sources: graph.sources,
            symbols: None,
        };
    }
    let root = graph.add(root);
    graph.visit(root);
    let Graph {
        nodes,
        sources,
        mut diags,
        order,
        ..
    } = graph;
    if diags.iter().any(Diagnostic::is_error) {
        diags.sort_by_key(|d| d.span.lo);
        return Compiled {
            program: None,
            diagnostics: diags,
            sources,
            symbols: None,
        };
    }
    // The modules in evaluation order, with their imports as positions in it.
    let mut position = vec![0; nodes.len()];
    for (at, &node) in order.iter().enumerate() {
        position[node] = at;
    }
    let modules: Vec<ModuleAst> = order
        .iter()
        .map(|&i| ModuleAst {
            ast: &nodes[i].ast,
            entry: entry && i == root,
            imports: nodes[i].imports.iter().map(|t| t.map(|t| position[t])).collect(),
        })
        .collect();
    let (mut program, mut lowered, symbols) = lower_modules(&modules, sources.text());
    diags.append(&mut lowered);
    // An imported file reads data relative to itself.
    for input in &mut program.inputs {
        if let Some(file) = sources.file_index(input.span.lo)
            && file != root
            && input.path != "-"
        {
            input.path = loader.data_path(&nodes[file].identity, &input.path);
        }
    }
    diags.sort_by_key(|d| d.span.lo);
    let program = (!diags.iter().any(Diagnostic::is_error)).then_some(program);
    Compiled {
        program,
        diagnostics: diags,
        sources,
        symbols: Some(symbols),
    }
}

struct Node {
    identity: String,
    name: String,
    ast: ast::Program,
    /// For each of its imports, in order, the file it names, if it loaded.
    imports: Vec<Option<usize>>,
    finished: bool,
}

struct Graph<'l> {
    loader: &'l mut dyn Loader,
    nodes: Vec<Node>,
    by_identity: FxHashMap<String, usize>,
    sources: SourceMap,
    diags: Vec<Diagnostic>,
    /// The files being visited, from the entry.
    stack: Vec<usize>,
    /// Finished files, each after the files it imports.
    order: Vec<usize>,
}

impl Graph<'_> {
    /// Parse a file, at its place in the source map.
    fn add(&mut self, source: Source) -> usize {
        let base = self
            .sources
            .add(SourceFile::new(source.name.clone(), source.text.clone()));
        let (ast, mut diags) = parse_program_at(&source.text, base);
        self.diags.append(&mut diags);
        self.by_identity.insert(source.identity.clone(), self.nodes.len());
        self.nodes.push(Node {
            identity: source.identity,
            name: source.name,
            ast,
            imports: Vec::new(),
            finished: false,
        });
        self.nodes.len() - 1
    }

    /// Load what file `i` imports, depth first, then finish it.
    fn visit(&mut self, i: usize) {
        self.stack.push(i);
        let imports: Vec<(Span, Span, String, bool)> = self.nodes[i]
            .ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Import(import) => Some((
                    import.span,
                    import.path_span,
                    import.path.clone(),
                    import.names.is_none(),
                )),
                _ => None,
            })
            .collect();
        for (span, path_span, path, bare) in imports {
            let target = if bare {
                self.diags.push(
                    Diagnostic::error(span, "a bare `import` isn't supported yet")
                        .with_help("name what you import: `import { name } from \"…\"`"),
                );
                None
            } else {
                self.resolve(i, &path, path_span)
            };
            self.nodes[i].imports.push(target);
        }
        self.stack.pop();
        self.nodes[i].finished = true;
        self.order.push(i);
    }

    /// The file that file `i` imports as `path`, loaded, parsed and visited
    /// unless it was already.
    fn resolve(&mut self, i: usize, path: &str, span: Span) -> Option<usize> {
        let importer = self.nodes[i].identity.clone();
        let source = match self.loader.load(&importer, path) {
            Ok(source) => source,
            Err(message) => {
                self.diags
                    .push(Diagnostic::error(span, format!("can't import \"{path}\": {message}")));
                return None;
            }
        };
        if let Some(&k) = self.by_identity.get(&source.identity) {
            if !self.nodes[k].finished {
                let start = self.stack.iter().position(|&s| s == k).unwrap_or(0);
                let chain: Vec<&str> = self.stack[start..]
                    .iter()
                    .chain([&k])
                    .map(|&s| self.nodes[s].name.as_str())
                    .collect();
                self.diags.push(
                    Diagnostic::error(span, format!("this import makes a cycle: {}", chain.join(" → "))).with_help(
                        "files can't import each other, directly or not; move what they share into a file of its own",
                    ),
                );
                return None;
            }
            return Some(k);
        }
        let problem = if self.stack.len() >= MAX_DEPTH {
            Some(format!("imports nest more than {MAX_DEPTH} files deep"))
        } else if self.nodes.len() >= MAX_MODULES {
            Some(format!("the program imports more than {MAX_MODULES} files"))
        } else if source.text.len() > MAX_SOURCE {
            Some(format!("\"{path}\" is larger than {MAX_SOURCE} bytes"))
        } else if self.sources.text().len() + source.text.len() > MAX_TOTAL_SOURCE {
            Some(format!(
                "the program's files are larger than {MAX_TOTAL_SOURCE} bytes together"
            ))
        } else {
            None
        };
        if let Some(problem) = problem {
            self.diags.push(Diagnostic::error(span, problem));
            return None;
        }
        let k = self.add(source);
        self.visit(k);
        Some(k)
    }
}
