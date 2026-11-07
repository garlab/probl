use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "probl-model-tests-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, source: &str) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_probl"))
            .arg("test")
            .args(args)
            .current_dir(&self.0)
            .output()
            .unwrap()
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn command_runs_tests_independently_and_filters_names() {
    let files = Files::new();
    files.write("model.test.probl", "test \"good\" { expect d6 > 0 }\ntest \"wrong\" { expect false as \"counterexample\" }\ntest \"broken\" { expect missing }\ntest \"after\" { expect true }");
    let out = files.run(&["model.test.probl", "--today", "2026-11-06"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success());
    assert!(
        text.contains("2 passed · 1 failed · 1 errors · 0 inconclusive"),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("now 2026-11-06T00:00:00"));
    assert!(text.contains("counterexample"));
    assert!(files.run(&["model.test.probl", "--filter", "good"]).status.success());
    let empty = files.run(&["model.test.probl", "--filter", "absent"]);
    assert!(!empty.status.success());
    assert!(String::from_utf8_lossy(&empty.stderr).contains("no tests selected"));
}

#[test]
fn discovery_is_sorted_deduplicated_and_reads_inputs_next_to_modules() {
    let files = Files::new();
    files.write("lib/data.probl", "export let xs: list[int] = read(\"numbers.json\")");
    files.write("lib/numbers.json", "[1,2,3]");
    files.write(
        "b.test.probl",
        "import { xs } from \"./lib/data.probl\"\ntest \"data\" { expect len(xs) == 3 }",
    );
    files.write(
        "nested/a.test.probl",
        "test \"clock\" { expect now == '2026-11-06T00:00:00'; expect today == date(now) }",
    );
    let out = files.run(&[".", "b.test.probl", "--today", "2026-11-06"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("2 passed"), "{text}");
    assert!(text.find(":: data").unwrap() < text.find(":: clock").unwrap());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&files.0, files.0.join("cycle")).unwrap();
        assert!(files.run(&[".", "--today", "2026-11-06"]).status.success());
    }
}

#[test]
fn cancellation_and_empty_suites_do_not_pass() {
    let files = Files::new();
    assert!(!files.run(&[]).status.success());
    files.write("a.test.probl", "test \"ok\" { expect true }");
    let out = files.run(&["--timeout", "0"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("unrun"));
    assert!(
        !files
            .run(&["--today", "2026-11-06", "--now", "2026-11-07T00:00:00"])
            .status
            .success()
    );
}

#[test]
fn check_compiles_test_files_and_run_points_to_test() {
    let files = Files::new();
    files.write(
        "model.test.probl",
        "let n = 2\ntest \"a\" { expect n == 2 }\ntest \"b\" { expect n > 1 }",
    );
    let probl = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_probl"))
            .args(args)
            .current_dir(&files.0)
            .output()
            .unwrap()
    };
    let out = probl(&["check", "model.test.probl"]);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && text.contains("model.test.probl: ok (2 tests)"),
        "{text}"
    );
    let out = probl(&["run", "model.test.probl"]);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && text.contains("probl test model.test.probl"),
        "{text}"
    );
    files.write(
        "broken.test.probl",
        "report 1\ntest \"a\" { expect true }\ntest \"b\" { expect true }",
    );
    let out = probl(&["check", "broken.test.probl"]);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success() && text.matches("reports only in its tests").count() == 1,
        "{text}"
    );
}

#[test]
fn output_names_files_relatively_and_failures_by_their_share() {
    let files = Files::new();
    files.write(
        "tests/dice.test.probl",
        "1 + 1\ntest \"six\" { let r ~ d6\n expect r < 6 as \"never a six\" }\ntest \"other\" { expect true }",
    );
    let out = files.run(&["tests"]);
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(stdout.contains("FAIL tests/dice.test.probl :: six"), "{stdout}");
    assert!(
        stdout.contains("FAIL: never a six: false for 16.67% of the weight"),
        "{stdout}"
    );
    // The file's warning, once.
    assert_eq!(stderr.matches("this value isn't used").count(), 1, "{stderr}");
}
