//! Reading data from the command line (docs/data-input.md), with the real
//! `probl` binary.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A fresh directory with these files.
fn dir(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("probl-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

fn probl(args: &[&str], cwd: &Path, stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut pipe = child.stdin.take().unwrap();
    if let Some(text) = stdin {
        pipe.write_all(text.as_bytes()).unwrap();
    }
    drop(pipe);
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

const MODEL: &str = "type Day = { day: date, signups: int }\n\
                     let pilot: list[Day] = read(\"data/pilot.csv\")\n\
                     report sum(map(pilot, d -> d.signups)) as \"total\"\n";

#[test]
fn data_is_read_next_to_the_program() {
    let root = dir(
        "next-to",
        &[
            ("model/m.probl", MODEL),
            (
                "model/data/pilot.csv",
                "day,visitors,signups\n2026-09-01,250,9\n2026-09-02,250,6\n",
            ),
        ],
    );
    // From another directory: paths are relative to the program.
    let out = probl(&["run", "model/m.probl", "--stats"], &root, None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("total    15"), "{}", text(&out.stdout));
    assert!(
        text(&out.stderr).contains("pilot.csv · 55 bytes · sha256 "),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn data_can_come_from_a_pipe() {
    let root = dir(
        "pipe",
        &[(
            "counts.probl",
            "let counts: list[int] = read(\"-\")\nreport sum(counts) as \"total\"\n",
        )],
    );
    let out = probl(&["run", "counts.probl"], &root, Some("9\n6\n\n11\n"));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stdout).contains("total    26"), "{}", text(&out.stdout));
}

#[test]
fn bad_data_is_an_error_at_the_read() {
    let root = dir(
        "bad",
        &[
            ("m.probl", MODEL),
            ("data/pilot.csv", "day,signups\n2026-09-01,9\n2026-09-02,n/a\n"),
        ],
    );
    let out = probl(&["run", "m.probl"], &root, None);
    assert_eq!(out.status.code(), Some(1));
    let err = text(&out.stderr);
    assert!(
        err.contains("data/pilot.csv, line 3, column `signups`: `n/a` isn't an int"),
        "{err}"
    );
    assert!(err.contains("m.probl:2:"), "{err}");
}

#[test]
fn check_reads_data_only_when_asked() {
    let root = dir("check", &[("m.probl", MODEL)]);
    let out = probl(&["check", "m.probl"], &root, None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let out = probl(&["check", "--data", "m.probl"], &root, None);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("data/pilot.csv: can't read it: there's no such file"));
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("data/pilot.csv"), "day,signups\n2026-09-01,9\n").unwrap();
    let out = probl(&["check", "--data", "m.probl"], &root, None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("ok, and so is its data (1 file)"));
}

#[test]
fn schema_suggests_a_type() {
    let root = dir("schema", &[("pilot.csv", "Day,Daily sign-ups\n2026-09-01,9\n")]);
    let out = probl(&["schema", "pilot.csv"], &root, None);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        text(&out.stdout),
        "type Pilot = { day: date, daily_sign_ups: int }\n# let pilot: list[Pilot] = read(\"pilot.csv\")\n"
    );
    let out = probl(&["schema", "-", "--format", "json"], &root, Some("{\"a\": [1, 2.5]}"));
    assert_eq!(
        text(&out.stdout),
        "type Data = { a: list[float] }\n# let data: Data = read(\"-\")\n"
    );
    let out = probl(&["schema", "pilot.xlsx"], &root, None);
    assert!(text(&out.stderr).contains("--format"));
}

#[test]
fn inputs_are_limited() {
    let root = dir(
        "limits",
        &[
            ("m.probl", MODEL),
            ("data/pilot.csv", "day,signups\n2026-09-01,9\n2026-09-02,6\n"),
        ],
    );
    let out = probl(&["run", "m.probl", "--max-input", "20"], &root, None);
    assert!(!out.status.success());
    assert!(
        text(&out.stderr).contains("the data is more than"),
        "{}",
        text(&out.stderr)
    );
}
