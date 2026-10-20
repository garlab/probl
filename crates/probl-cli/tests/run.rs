//! `probl run`'s options for sampling, with the real `probl` binary.

use std::process::Command;

/// Run `probl run` on `benches/ab_test.probl` with these extra arguments:
/// its output, and what it wrote to standard error.
fn ab_test(args: &[&str]) -> (String, String) {
    let model = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benches/ab_test.probl");
    let output = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(["run", model, "--runs", "2000", "--stats"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn conjugate_priors_are_updated_exactly_unless_turned_off() {
    let (out, stats) = ab_test(&[]);
    assert!(out.contains("(± 0.00%) · effective sample size 2,000\n"), "{out}");
    assert!(
        stats.contains("exact updates: `a` (line 10) · 2,000 draws delayed · 60,000 observations · drawn 2,000 times"),
        "{stats}"
    );
    let (out, stats) = ab_test(&["--no-conjugate"]);
    assert!(!out.contains("effective sample size 2,000\n"), "{out}");
    assert!(
        stats.contains("exact updates: `b` (line 11) · 0 draws delayed · 0 observations · drawn 0 times"),
        "{stats}"
    );
}

#[test]
fn the_repl_shows_each_input_s_reports_whole() {
    use std::io::Write;
    let mut repl = Command::new(env!("CARGO_BIN_EXE_probl"))
        .arg("repl")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = "for k in 1..3 { report k * 2 by k }\n\"a {1}\"\nd6 > 3\n";
    repl.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = repl.wait_with_output().unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    // A table keeps all of its rows, and an expression's text is its label,
    // braces and all.
    for line in [
        "k * 2\n",
        "  1    2",
        "  2    4",
        "  3    6",
        "\"a {1}\"    a 1\n",
        "d6 > 3    50.00%\n",
    ] {
        assert!(out.contains(line), "{line:?} in\n{out}");
    }
}

/// Run `probl run` on `source`, written to a file of its own: the exit
/// status, the output and what it wrote to standard error.
fn run_source(name: &str, source: &str, args: &[&str]) -> (Option<i32>, String, String) {
    let dir = std::env::temp_dir().join(format!("probl-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_probl"))
        .arg("run")
        .arg(&path)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn report_summary_flags_select_ordered_columns_by_id() {
    let source = "for k in 0..2 {report one_of([0:50%,10:50%]) by k as \"same\"}; report true as \"same\"";
    let (code, out, err) = run_source(
        "summary.probl",
        source,
        &["--summary", "0=q:0.5,median,mean", "--summary", "1=moments"],
    );
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("50%") && out.contains("median") && out.contains("mean"),
        "{out}"
    );
    assert!(out.contains("100.00%"));
    assert!(!out.contains("95%"));
    for args in [
        vec!["--summary", "0=q:NaN"],
        vec!["--summary", "2=forecast"],
        vec!["--summary", "0=mean,mean"],
        vec!["--summary", "0=forecast", "--summary", "0=moments"],
    ] {
        let (code, _, err) = run_source("bad-summary.probl", source, &args);
        assert_ne!(code, Some(0));
        assert!(err.contains("summary") || err.contains("quantile"), "{err}");
    }
}

#[test]
fn a_partial_result_prints_the_other_worlds_and_exits_with_3() {
    let source = "let x ~ d6 - 1\nlet y = 1 / x\nreport y\n";
    // Enumeration stops at the first fault.
    let (code, out, err) = run_source("total.probl", source, &[]);
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(out.is_empty() && err.contains("division by zero"), "{out}{err}");
    // Partial: the other worlds' reports, then where the others failed.
    let (code, out, err) = run_source("partial.probl", source, &["--on-error", "partial"]);
    assert_eq!(code, Some(3), "{out}{err}");
    assert_eq!(
        out,
        "enumerated · partial result · 16.67% failed\n\n\
         y    mean 0.46 · sd 0.29 · 5% 0.20 · median 0.33 · 95% 1.00 (reached in 83.33% of worlds)\n\n\
         failed\n  line 2: division by zero    16.67%\n\n"
    );
    assert!(
        err.contains("it failed in 16.67% of the worlds; the other worlds finished"),
        "{err}"
    );
    // Sampling is partial by default, unless asked otherwise.
    let (code, out, _) = run_source("sampled.probl", source, &["--runs", "1000"]);
    assert_eq!(code, Some(3));
    assert!(
        out.contains("· partial result · ") && out.contains(" runs failed"),
        "{out}"
    );
    let (code, out, _) = run_source("sampled.probl", source, &["--runs", "1000", "--on-error", "total"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    // Every world failed: an error, with what was reported before.
    let (code, out, err) = run_source(
        "all.probl",
        "@on_error partial\nlet x ~ d6\nreport x\nlet y = x / 0\n",
        &[],
    );
    assert_eq!(code, Some(1), "{out}{err}");
    assert!(
        out.contains("100.00% failed") && err.contains("no world finished"),
        "{out}{err}"
    );
}

#[test]
fn env_is_the_process_s_environment_with_env_flags_on_top() {
    let source = "report env.get(\"PROBL_TEST_REGION\", \"eu\") as \"region\"\n\
                  report env.get(\"PROBL_TEST_OTHER\", \"none\") as \"other\"";
    let dir = std::env::temp_dir().join(format!("probl-cli-env-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("env.probl");
    std::fs::write(&path, source).unwrap();
    let probl = |region: Option<&str>, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_probl"));
        command
            .arg("run")
            .arg(&path)
            .args(args)
            .env_remove("PROBL_TEST_REGION")
            .env_remove("PROBL_TEST_OTHER");
        if let Some(region) = region {
            command.env("PROBL_TEST_REGION", region);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    };
    let out = probl(None, &[]);
    assert!(out.contains("region    eu") && out.contains("other     none"), "{out}");
    let out = probl(Some("us"), &[]);
    assert!(out.contains("region    us"), "{out}");
    // `--env` takes the place of the process's variable, or adds one.
    let out = probl(
        Some("us"),
        &["--env", "PROBL_TEST_REGION=apac", "--env", "PROBL_TEST_OTHER=x=y"],
    );
    assert!(out.contains("region    apac") && out.contains("other     x=y"), "{out}");
    let output = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(["run", path.to_str().unwrap(), "--env", "PROBL_TEST_REGION"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected NAME=VALUE"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_repl_has_the_environment_and_can_t_change_it() {
    use std::io::Write;
    let mut repl = Command::new(env!("CARGO_BIN_EXE_probl"))
        .arg("repl")
        .env("PROBL_TEST_REGION", "us")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let input = "env[\"PROBL_TEST_REGION\"]\nenv = [\"a\": \"b\"]\nenv.get(\"PROBL_TEST_REGION\", \"eu\")\n";
    repl.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let output = repl.wait_with_output().unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(out.contains("env[\"PROBL_TEST_REGION\"]    us"), "{out}");
    assert!(out.contains("env.get(\"PROBL_TEST_REGION\", \"eu\")    us"), "{out}");
    assert!(err.contains("can't assign to `env`: it's a constant"), "{err}");
}

#[test]
fn collection_flags_are_explicit_and_checked() {
    let source = "@mode sample(runs:100,seed:42)\nlet x~d6; report x";
    let (code, out, err) = run_source("collection.probl", source, &["--collect", "0=moments"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("mean") && out.contains("sd") && !out.contains("median"),
        "{out}"
    );
    for args in [
        vec!["--collect", "0=bad"],
        vec!["--collect", "1=moments"],
        vec!["--collect", "0=moments", "--summary", "0=forecast"],
        vec!["--collect", "0=moments", "--collect", "0=exact"],
        vec!["--collect", "0=moments", "--mode", "enumerate"],
    ] {
        let (code, _, _) = run_source("bad-collection.probl", source, &args);
        assert_ne!(code, Some(0));
    }
}
