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
