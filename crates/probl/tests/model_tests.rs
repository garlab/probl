use probl::{MemoryFiles, MemorySources, Options, TestStatus, compile_tests};

fn run(source: &str, options: Options) -> Vec<probl::TestResult> {
    let suite = compile_tests("model.test.probl", source, &mut MemorySources::new())
        .unwrap_or_else(|e| panic!("{}", e.render(false)));
    suite
        .tests()
        .iter()
        .map(|test| test.run(&options, &mut MemoryFiles::new()))
        .collect()
}

#[test]
fn tests_check_the_final_population_without_conditioning_it() {
    let results = run(
        "test \"all worlds\" { let x ~ d2; expect x == 1; expect x == 2; expect x == x; expect d6 > 0 }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Fail, "{:?}", results[0].message);
    let assertions = &results[0].assertions;
    assert_eq!(
        assertions.iter().map(|a| a.status).collect::<Vec<_>>(),
        [TestStatus::Fail, TestStatus::Fail, TestStatus::Pass, TestStatus::Pass]
    );
    assert_eq!(assertions[0].false_probability, 0.5);
    assert_eq!(assertions[1].false_probability, 0.5);
}

#[test]
fn tiny_counterexamples_are_not_rounded_away() {
    let results = run(
        "test \"tiny\" { let x ~ d2; score if x == 1 { 1e-40 } else { 1 }; expect x == 2 as \"almost always\" }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Fail, "{:?}", results[0].message);
    assert!(results[0].assertions[0].false_probability > 0.0);
    assert!(results[0].assertions[0].false_probability < 1e-30);
    assert_eq!(results[0].assertions[0].label, "almost always");
    assert_eq!(results[0].assertions[0].file, "model.test.probl");
    let results = run(
        "test \"underflow\" { let x ~ d2; repeat 5 { score if x == 1 { 1e-100 } else { 1 } }; expect x == 2 }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Fail);
    assert_eq!(results[0].assertions[0].false_probability, 0.0);
    assert!(results[0].assertions[0].log_false_weight < -1000.0);
}

#[test]
fn fixtures_and_imports_are_fresh_and_correlated_for_each_test() {
    let mut sources: MemorySources = [("lib.probl", "export let roll ~ d2")].into_iter().collect();
    let suite = compile_tests("model.test.probl", "import { roll } from \"./lib.probl\"\nvar fixture = [roll]\ntest \"first\" { var local = fixture; local.push(3); expect len(local) == 2; expect fixture[0] == roll }\ntest \"second\" { expect len(fixture) == 1; expect fixture[0] == roll }\ntest \"readonly\" { fixture.push(3); expect true }", &mut sources).unwrap_or_else(|e| panic!("{}", e.render(false)));
    for _ in 0..2 {
        let results: Vec<_> = suite
            .tests()
            .iter()
            .map(|test| test.run(&Options::new(), &mut MemoryFiles::new()))
            .collect();
        assert_eq!(
            results.iter().map(|r| r.status).collect::<Vec<_>>(),
            [TestStatus::Pass, TestStatus::Pass, TestStatus::Error]
        );
    }
}

#[test]
fn runtime_errors_do_not_stop_independent_tests() {
    let results = run(
        "test \"fault\" { let x = 1 / 0; expect x > 0 }\ntest \"empty\" { observe false; expect true }\ntest \"good\" { expect true }",
        Options::new(),
    );
    assert_eq!(
        results.iter().map(|r| r.status).collect::<Vec<_>>(),
        [TestStatus::Error, TestStatus::Error, TestStatus::Pass]
    );
}

#[test]
fn assertions_are_read_only_even_through_function_values() {
    for setup_and_query in [
        "let f = () -> ~d6; expect f() > 0",
        "expect if 50% { true } else { false }",
        "expect { print(1); true }",
        "expect { observe true; true }",
        "expect { score 50%; true }",
        "expect simulate { true }",
        "expect [1].map(x -> simulate { x }) == [1]",
        "expect 99%",
        "expect 1",
        "expect mean(d6) == 3.5",
        "let query = mean; expect query(d6) == 3.5",
    ] {
        let results = run(&format!("test \"readonly\" {{ {setup_and_query} }}"), Options::new());
        assert_eq!(
            results[0].status,
            TestStatus::Error,
            "{setup_and_query}: {:?}",
            results[0].message
        );
        assert!(results[0].printed.is_empty());
    }
    let results = run(
        "fn pure(x) { var y = x; repeat 3 { y += 1 }; y }\ntest \"helpers\" { let x ~ d2; expect pure(x) == x + 3; report pure(x) as \"detail\" }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Pass, "{:?}", results[0].message);
    assert!(results[0].output.contains("detail"));
}

#[test]
fn unresolved_setup_and_nested_summaries_are_inconclusive() {
    let options = Options::new().epsilon(0.1);
    for source in [
        "test \"tail\" { var n = 0; while 50% { n += 1 }; expect n >= 0 }",
        "test \"summary\" { let d = simulate { var n = 0; while 50% { n += 1 }; n }; let m = mean(d); expect m >= 0 }",
        "test \"summary false\" { let d = simulate { var n = 0; while 50% { n += 1 }; n }; let m = mean(d); expect m < 0 }",
    ] {
        let results = run(source, options.clone());
        assert_eq!(
            results[0].status,
            TestStatus::Inconclusive,
            "{source}: {:?}",
            results[0].message
        );
    }
}

#[test]
fn test_syntax_is_explicit_and_assertions_are_terminal() {
    assert!(probl::compile("a.test.probl", "test \"not enabled\" { expect true }").is_err());
    for source in [
        "test \"\" { expect true }",
        "test \"same\" { expect true }\ntest \"same\" { expect true }",
        "test \"empty\" {}",
        "test \"late\" { expect true; let x = 1 }",
        "test \"nested\" { if true { expect true } }",
    ] {
        assert!(
            compile_tests("a.probl", source, &mut MemorySources::new()).is_err(),
            "{source}"
        );
    }
    let results = run("test \"return\" { if true { return 1 }; expect true }", Options::new());
    assert_eq!(results[0].status, TestStatus::Error);
}

#[test]
fn analytic_properties_use_almost_sure_checks_and_never_sample() {
    let results = run(
        "test \"continuous\" { let x ~ normal(0, 1); expect x == x; expect x != 0; expect x < 0; expect x > -9 }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Fail, "{:?}", results[0].message);
    assert_eq!(
        results[0].assertions.iter().map(|a| a.status).collect::<Vec<_>>(),
        [TestStatus::Pass, TestStatus::Pass, TestStatus::Fail, TestStatus::Fail]
    );
    assert_eq!(results[0].assertions[2].false_probability, 0.5);
    let rare_tail = results[0].assertions[3].false_probability;
    assert!(rare_tail > 0.0 && rare_tail < 1e-18, "{rare_tail}");
    let results = run(
        "test \"unrepresentable\" { let x ~ normal(0, 1); expect x < 9 }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Error);
    assert!(results[0].message.as_ref().unwrap().contains("nonzero tail"));
    let results = run(
        "test \"product\" { let x ~ normal(0, 1); let y ~ normal(0, 1); let z ~ normal(0, 1); let w ~ normal(0, 1); expect not (x < -20 and y < -20 and z < -20 and w < -20) }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Fail, "{:?}", results[0].message);
    assert_eq!(results[0].assertions[0].false_probability, 0.0);
    assert!(results[0].assertions[0].log_false_weight.is_finite());
    for options in [Options::new().runs(1).seed(12), Options::new().integrate(0.01)] {
        let result = run("test \"no approximation\" { expect true }", options).remove(0);
        assert_eq!(result.status, TestStatus::Error);
        assert_eq!(result.error_kind, Some(probl::ErrorKind::Unsupported));
    }
}

#[test]
fn incomplete_recipes_and_partial_faults_cannot_pass() {
    let results = run(
        "test \"recipe\" { let d = simulate { var n = 0; while 50% { n += 1 }; n }; expect d >= 0 }",
        Options::new().epsilon(0.1),
    );
    assert_eq!(results[0].status, TestStatus::Inconclusive, "{:?}", results[0].message);
    let results = run(
        "test \"recipe underflow\" { let d = simulate { let x ~ d2; repeat 5 { score if x == 1 { 1e-100 } else { 1 } }; x == 2 }; expect d }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Inconclusive, "{:?}", results[0].message);
    let results = run(
        "test \"analytic recipe underflow\" { let d = simulate { let x ~ normal(0, 1); let y ~ normal(0, 1); let z ~ normal(0, 1); let w ~ normal(0, 1); not (x < -20 and y < -20 and z < -20 and w < -20) }; expect d }",
        Options::new(),
    );
    assert_eq!(results[0].status, TestStatus::Error, "{:?}", results[0].message);
    assert_eq!(results[0].error_kind, Some(probl::ErrorKind::Unsupported));
    let results = run(
        "test \"fault\" { let x ~ d2; let y = 1 / (x - 1); expect y == 1 }",
        Options::new().on_error(probl::FailureMode::Partial),
    );
    assert_eq!(results[0].status, TestStatus::Error);
}

#[test]
fn tests_preserve_source_snapshots_data_and_clock_metadata() {
    let mut sources: MemorySources = [("lib.probl", "export let x = 1")].into_iter().collect();
    let suite = compile_tests("arbitrary-name", "import { x } from \"./lib.probl\"\nlet xs: list[int] = read(\"input.json\")\ntest \"snapshot\" { let total = sum(xs); expect x == 1; expect total == 3; expect now == '2026-11-06T00:00:00' }", &mut sources).unwrap_or_else(|e| panic!("{}", e.render(false)));
    sources.insert("lib.probl", "export let x = 9");
    let mut files: MemoryFiles = [("input.json", b"[1,2]".to_vec())].into_iter().collect();
    let options = Options::new().today("2026-11-06".parse().unwrap());
    let result = suite.tests()[0].run(&options, &mut files);
    assert_eq!(result.status, TestStatus::Pass, "{:?}", result.message);
    assert_eq!(result.now.unwrap().to_string(), "2026-11-06T00:00:00");
    assert_eq!(result.today.unwrap().to_string(), "2026-11-06");
}
