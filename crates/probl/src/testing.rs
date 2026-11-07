//! Isolated, enumerated model tests. Assertions consume typed, unrounded
//! fact weights, never rendered report text.

use crate::{Diagnostic, Error, Files, Options, Outcome, Program, Sources};
use probl_syntax::SourceMap;
use std::sync::Arc;

/// Compile explicit test syntax. Ordinary `compile` never enables it based
/// on a filename. A suite owns its source snapshot; executing it doesn't
/// reload modules.
pub fn compile_tests(name: &str, source: &str, sources: &mut dyn Sources) -> Result<TestSuite, Error> {
    let root = probl_sema::Source {
        identity: sources.entry(name),
        name: name.to_string(),
        text: source.to_string(),
    };
    let compiled = probl_sema::modules::compile_tests(root, &mut crate::sources::Load(sources));
    if compiled.diagnostics.iter().any(probl_syntax::Diagnostic::is_error) {
        return Err(Error::compile(compiled.diagnostics, &Arc::new(compiled.sources)));
    }
    let compiled_source_bytes = compiled.tests.iter().map(|(_, test)| test.sources.text().len()).sum();
    let mut shared_sources = None;
    let tests = compiled
        .tests
        .into_iter()
        .zip(compiled.assertions)
        .map(|((name, compiled), assertions)| {
            // Every test has the same root and frozen import graph.
            let sources = shared_sources.get_or_insert_with(|| Arc::new(compiled.sources)).clone();
            let program = match compiled.program {
                Some(ir) => Ok(Program {
                    ir: Arc::new(ir),
                    warnings: compiled
                        .diagnostics
                        .into_iter()
                        .map(|d| Diagnostic::new(d, &sources))
                        .collect(),
                    sources,
                }),
                None => Err(Error::compile(compiled.diagnostics, &sources)),
            };
            TestCase {
                name,
                program,
                assertions,
            }
        })
        .collect();
    Ok(TestSuite {
        tests,
        compiled_source_bytes,
    })
}

#[derive(Debug)]
pub struct TestSuite {
    tests: Vec<TestCase>,
    compiled_source_bytes: usize,
}

impl TestSuite {
    pub fn tests(&self) -> &[TestCase] {
        &self.tests
    }

    /// Source bytes processed across the independently compiled tests,
    /// including imports. Hosts can use this to bound a multi-file suite.
    pub fn compiled_source_bytes(&self) -> usize {
        self.compiled_source_bytes
    }
}

/// One independent test, including its compile error if only this test
/// could not be compiled. Other tests in the suite remain executable.
#[derive(Debug)]
pub struct TestCase {
    name: String,
    program: Result<Program, Error>,
    assertions: Vec<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestStatus {
    Pass,
    Fail,
    Error,
    Inconclusive,
}

#[derive(Clone, Debug)]
pub struct AssertionResult {
    pub label: String,
    pub file: String,
    pub span: std::ops::Range<usize>,
    pub status: TestStatus,
    /// Counterexample share among resolved outcomes. May underflow to zero;
    /// `status` is determined from the original positive weight.
    pub false_probability: f64,
    /// Natural log of unnormalized counterexample weight; -infinity for zero.
    pub log_false_weight: f64,
}

#[derive(Clone, Debug)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub assertions: Vec<AssertionResult>,
    pub diagnostics: Vec<Diagnostic>,
    pub message: Option<String>,
    pub error_kind: Option<crate::ErrorKind>,
    pub printed: Vec<String>,
    /// Explicit diagnostic reports only; assertions aren't rendered here.
    pub output: String,
    pub now: Option<crate::DateTime>,
    pub today: Option<crate::Date>,
}

impl TestCase {
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Run with fresh model state. Supply one pinned clock/environment
    /// snapshot in `options` for all tests in a suite. Test execution always
    /// enumerates, with exact report retention and no sampling fallback.
    pub fn run(&self, options: &Options, files: &mut dyn Files) -> TestResult {
        let mut result = TestResult {
            name: self.name.clone(),
            status: TestStatus::Error,
            assertions: Vec::new(),
            diagnostics: Vec::new(),
            message: None,
            error_kind: None,
            printed: Vec::new(),
            output: String::new(),
            now: None,
            today: None,
        };
        let program = match &self.program {
            Ok(program) => program,
            Err(error) => {
                result.record_error(error);
                return result;
            }
        };
        result.diagnostics.extend_from_slice(program.warnings());
        let requested = options.engine(&program.ir);
        if matches!(requested.mode, Some(probl_sema::ir::Mode::Sample { .. })) || requested.integrate.is_some() {
            result.error_kind = Some(crate::ErrorKind::Unsupported);
            result.message = Some("model tests require enumeration without sampling or numerical integration".into());
            return result;
        }
        let mut options = options.clone().enumerate();
        options.report_summaries.clear();
        options.report_collections.clear();
        options.data = None;
        if program.reads_data() {
            match program.load(files, &options) {
                Ok(data) => options = options.data(data),
                Err(error) => {
                    result.record_error(&error);
                    return result;
                }
            }
        }
        let mut printed = Vec::new();
        let execution = program.run_with(&options, &mut |line| printed.push(line.to_string()));
        match execution {
            Ok(outcome) => self.collect(program, &outcome, &mut result),
            Err(error) => {
                if let Some(outcome) = error.partial() {
                    self.collect(program, outcome, &mut result);
                }
                result.record_error(&error);
            }
        }
        result.printed = printed;
        result
    }

    fn collect(&self, program: &Program, outcome: &Outcome, result: &mut TestResult) {
        result.now = outcome.now();
        result.today = outcome.today();
        let complete = outcome.unresolved().is_zero() && !outcome.incomplete_inference;
        result.status = if complete && outcome.finished() {
            TestStatus::Pass
        } else {
            TestStatus::Inconclusive
        };
        if !outcome.finished() {
            result.message = Some("no positive-weight world finished the test".into());
        } else if !complete {
            result.message =
                Some("the test or an inference used during setup left weight unresolved or unrepresentable".into());
        }
        for (i, &assertion) in self.assertions.iter().enumerate() {
            if !assertion {
                result.output.push_str(&outcome.render(i..i + 1));
                continue;
            }
            let site = &program.ir.reports[i];
            let (file, span) = location(&program.sources, site.span);
            let Some((false_weight, total, recipe_complete)) = outcome.test_facts[i] else {
                if result.status == TestStatus::Pass {
                    result.status = TestStatus::Inconclusive;
                }
                result.assertions.push(AssertionResult {
                    label: site.label.clone(),
                    file,
                    span,
                    status: TestStatus::Inconclusive,
                    false_probability: 0.0,
                    log_false_weight: f64::NEG_INFINITY,
                });
                continue;
            };
            // A setup summary may have erased its missing-mass metadata.
            // Until query dependence is tracked, none of its downstream
            // predicates can be certified as a definite counterexample.
            let status = if outcome.incomplete_inference {
                TestStatus::Inconclusive
            } else if !false_weight.is_zero() {
                TestStatus::Fail
            } else if complete && recipe_complete {
                TestStatus::Pass
            } else {
                TestStatus::Inconclusive
            };
            if status == TestStatus::Fail {
                result.status = TestStatus::Fail;
            } else if status == TestStatus::Inconclusive && result.status == TestStatus::Pass {
                result.status = status;
            }
            result.assertions.push(AssertionResult {
                label: site.label.clone(),
                file,
                span,
                status,
                false_probability: false_weight.ratio(total),
                log_false_weight: false_weight.ln(),
            });
        }
    }
}

impl TestResult {
    fn record_error(&mut self, error: &Error) {
        self.status = TestStatus::Error;
        self.message = Some(error.to_string());
        self.error_kind = Some(error.kind());
        self.diagnostics.extend_from_slice(error.diagnostics());
    }
}

fn location(sources: &SourceMap, span: probl_syntax::Span) -> (String, std::ops::Range<usize>) {
    sources.locate(span).map_or_else(
        || (String::new(), 0..0),
        |(file, span)| (file.name.clone(), span.range()),
    )
}
