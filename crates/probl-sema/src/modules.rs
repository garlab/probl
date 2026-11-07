//! Loading a program's files: its entry and every file it imports, each once,
//! in evaluation order (docs/design/imports-and-tests.md).

use crate::ir;
use crate::lower::{ModuleAst, lower_model_test};
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
    compile(entry, loader, true, None)
}

/// Compile a file as it's compiled when another file imports it, with the
/// files it imports: to check it on its own, as an editor does. It can't
/// `report` or set pragmas, and its program is only for checking.
pub fn compile_module(source: Source, loader: &mut dyn Loader) -> Compiled {
    compile(source, loader, false, None)
}

/// A test source's independently compiled tests, in declaration order.
pub struct CompiledTests {
    pub tests: Vec<(String, Compiled)>,
    pub assertions: Vec<Vec<bool>>,
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

/// Compile test syntax explicitly. Dependencies are frozen for the suite,
/// and each test gets a fresh program containing the fixtures and its body.
pub fn compile_tests(source: Source, loader: &mut dyn Loader) -> CompiledTests {
    let sources = SourceMap::single(SourceFile::new(source.name.clone(), source.text.clone()));
    let (fixtures, tests, mut diagnostics) = if source.text.len() > MAX_SOURCE {
        (
            ast::Program {
                pragmas: Vec::new(),
                items: Vec::new(),
            },
            Vec::new(),
            vec![Diagnostic::error(Span::default(), "the test source is too large")],
        )
    } else {
        probl_syntax::parser::parse_tests_at(&source.text, 0)
    };
    if tests.is_empty() {
        diagnostics.push(Diagnostic::error(
            Span::default(),
            "a test source needs at least one test",
        ));
    }
    if tests.len() > 1000 {
        diagnostics.push(Diagnostic::error(
            Span::default(),
            "a test source may contain at most 1,000 tests",
        ));
    }
    if diagnostics.iter().any(Diagnostic::is_error) {
        return CompiledTests {
            tests: Vec::new(),
            assertions: Vec::new(),
            diagnostics,
            sources,
        };
    }
    struct Snapshot<'a> {
        loader: &'a mut dyn Loader,
        paths: FxHashMap<(String, String), Result<Source, String>>,
        identities: FxHashMap<String, Source>,
    }
    impl Loader for Snapshot<'_> {
        fn load(&mut self, importer: &str, path: &str) -> Result<Source, String> {
            let key = (importer.to_string(), path.to_string());
            if let Some(source) = self.paths.get(&key) {
                return source.clone();
            }
            let loaded = self
                .loader
                .load(importer, path)
                .map(|source| self.identities.entry(source.identity.clone()).or_insert(source).clone());
            self.paths.insert(key, loaded.clone());
            loaded
        }
        fn data_path(&mut self, importer: &str, path: &str) -> String {
            self.loader.data_path(importer, path)
        }
    }
    let mut loader = Snapshot {
        loader,
        paths: FxHashMap::default(),
        identities: FxHashMap::default(),
    };
    let assertions = tests
        .iter()
        .map(|test| test.assertions.iter().map(|a| !a.report).collect())
        .collect();
    let mut lowered = Vec::new();
    let mut source_work = 0;
    for test in &tests {
        let compiled = compile(source.clone(), &mut loader, false, Some((&fixtures, test)));
        source_work += compiled.sources.text().len();
        if source_work > 64 * 1024 * 1024 {
            diagnostics.push(Diagnostic::error(
                test.span,
                "compiling this test suite exceeds 64 MiB of source across its tests",
            ));
            break;
        }
        lowered.push((test.name.clone(), compiled));
    }
    let tests = lowered;
    CompiledTests {
        tests,
        assertions,
        diagnostics,
        sources,
    }
}

/// Compile the file `root` and the files it imports; `root` is the entry
/// when `entry` is true.
fn compile(
    root: Source,
    loader: &mut dyn Loader,
    entry: bool,
    test: Option<(&ast::Program, &ast::ModelTest)>,
) -> Compiled {
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
    let root = match test {
        Some((ast, _)) => graph.add_parsed(root, ast.clone()),
        None => graph.add(root),
    };
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
    let (mut program, mut lowered, symbols) = lower_model_test(&modules, sources.text(), test.map(|(_, test)| test));
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
        let base = self.sources.text().len() as u32 + u32::from(!self.nodes.is_empty());
        let (ast, mut diags) = parse_program_at(&source.text, base);
        self.diags.append(&mut diags);
        self.add_parsed(source, ast)
    }

    fn add_parsed(&mut self, source: Source, ast: ast::Program) -> usize {
        self.sources
            .add(SourceFile::new(source.name.clone(), source.text.clone()));
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
        if path.ends_with(".test.probl") {
            self.diags.push(
                Diagnostic::error(span, "test sources can't be imported")
                    .with_help("put shared helpers in an ordinary .probl module"),
            );
            return None;
        }
        let importer = self.nodes[i].identity.clone();
        let source = match self.loader.load(&importer, path) {
            Ok(source) => source,
            Err(message) => {
                self.diags
                    .push(Diagnostic::error(span, format!("can't import \"{path}\": {message}")));
                return None;
            }
        };
        if source.identity.ends_with(".test.probl") {
            self.diags
                .push(Diagnostic::error(span, "test sources can't be imported"));
            return None;
        }
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
