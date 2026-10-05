//! Compiling a program, loading its data and running it.

use crate::files::{Data, Files, Resolve};
use crate::{Diagnostic, Error, Options, Outcome};
use probl_sema::ir;
use probl_syntax::SourceFile;
use std::sync::Arc;

/// Compile a program. `name` is what its diagnostics call it, such as its
/// file's name. Warnings come with the program; errors come instead of it, as
/// an error of kind [`Compile`](crate::ErrorKind::Compile).
pub fn compile(name: &str, source: &str) -> Result<Program, Error> {
    let file = Arc::new(SourceFile::new(name, source));
    let (program, diagnostics) = probl_sema::compile(source);
    match program {
        Some(ir) => Ok(Program {
            ir: Arc::new(ir),
            warnings: diagnostics.into_iter().map(|d| Diagnostic::new(d, &file)).collect(),
            file,
        }),
        None => Err(Error::compile(diagnostics, &file)),
    }
}

/// A compiled program. Cloning it is cheap, and it can run any number of
/// times, from any thread.
#[derive(Clone)]
pub struct Program {
    ir: Arc<ir::Program>,
    file: Arc<SourceFile>,
    warnings: Vec<Diagnostic>,
}

/// How a program runs: its `@mode`, or what [`Options`] choose instead.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The default: enumerate, for now. Later, it may sample a model that
    /// can't be enumerated, and say so.
    Auto,
    /// Follow every random choice, exactly.
    Enumerate,
    /// Sample this many runs, from this seed.
    #[non_exhaustive]
    Sample { runs: u64, seed: u64 },
    /// Beam search: designed, not built yet. Running it is an error of kind
    /// [`Unsupported`](crate::ErrorKind::Unsupported).
    #[non_exhaustive]
    Beam { worlds: u64 },
    /// Particles: designed, not built yet. Running it is an error of kind
    /// [`Unsupported`](crate::ErrorKind::Unsupported).
    #[non_exhaustive]
    Particles { runs: u64, seed: u64 },
}

impl Program {
    /// What the compiler warned about.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Its `@mode`, which [`Options`] can override.
    pub fn mode(&self) -> Mode {
        match self.ir.settings.mode {
            ir::Mode::Auto => Mode::Auto,
            ir::Mode::Enumerate => Mode::Enumerate,
            ir::Mode::Sample { runs, seed } => Mode::Sample { runs, seed },
            ir::Mode::Beam { worlds } => Mode::Beam { worlds },
            ir::Mode::Particles { runs, seed } => Mode::Particles { runs, seed },
        }
    }

    /// Whether it uses `read`, and so needs [`load`](Program::load) before
    /// it runs.
    pub fn reads_data(&self) -> bool {
        !self.ir.inputs.is_empty()
    }

    /// Read its data from `files`, with the limits and the cancellation in
    /// `options`. Errors are about a `read(…)` call, and say where in the
    /// data they are.
    pub fn load(&self, files: &mut dyn Files, options: &Options) -> Result<Data, Error> {
        let mut snapshots = probl_engine::data::Snapshots::default();
        let cancel = options.cancel.as_ref().map(|c| c.flag());
        probl_engine::data::load(
            &self.ir,
            &mut Resolve(files),
            &mut snapshots,
            &options.limits.input(),
            cancel,
        )
        .map(Data::new)
        .map_err(|e| Error::runtime(e, &self.file))
    }

    /// Run it. What it `print`s is collected in the outcome.
    pub fn run(&self, options: &Options) -> Result<Outcome, Error> {
        let mut printed = Vec::new();
        let mut outcome = {
            let mut print = |line: &str| printed.push(line.to_string());
            self.run_with(options, &mut print)?
        };
        outcome.printed = printed;
        Ok(outcome)
    }

    /// Run it, giving each line it `print`s to `print` as it's printed.
    pub fn run_with(&self, options: &Options, print: &mut (dyn FnMut(&str) + Send)) -> Result<Outcome, Error> {
        let mut engine = options.engine(&self.ir);
        engine.inputs = self.inputs(options)?;
        #[cfg(target_arch = "wasm32")]
        let result = probl_engine::run_on_this_thread(&self.ir, &engine, print);
        #[cfg(not(target_arch = "wasm32"))]
        let result = probl_engine::run(&self.ir, &engine, print);
        let outcome = result.map_err(|e| Error::runtime(e, &self.file))?;
        Ok(Outcome::new(outcome, &self.ir, &self.file))
    }

    /// The data `options` give the run, which must be this program's.
    fn inputs(&self, options: &Options) -> Result<Option<Arc<probl_engine::data::Inputs>>, Error> {
        let span = self.ir.inputs.first().map(|i| i.span).unwrap_or_default();
        match &options.data {
            Some(data) if !data.inputs.fit(&self.ir) => Err(Error::usage(
                span,
                "the data was loaded for another program",
                "load this program's data with its `Program::load`",
                &self.file,
            )),
            Some(data) => Ok(Some(data.inputs.clone())),
            None if self.reads_data() => Err(Error::usage(
                span,
                "the program reads data, which wasn't loaded",
                "load it with `Program::load`, and give it to the run with `Options::data`",
                &self.file,
            )),
            None => Ok(None),
        }
    }

    /// How many reports it has, in [`Outcome::reports`].
    pub(crate) fn report_count(&self) -> usize {
        self.ir.reports.len()
    }
}

impl std::fmt::Debug for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("name", &self.file.name)
            .field("mode", &self.mode())
            .field("reports", &self.report_count())
            .field("warnings", &self.warnings)
            .finish()
    }
}
