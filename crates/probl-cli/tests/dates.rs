//! The executable captures a date once and accepts an explicit replay date.
use std::process::Command;

#[test]
fn now_snapshot_defaults_pins_normalizes_and_checks_conflicts() {
    let dir = std::env::temp_dir().join(format!("probl-now-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("now.probl");
    std::fs::write(&file, "report now\nreport today\nreport date(now)==today").unwrap();
    for (args, expected) in [
        (
            vec!["--now", "2027-01-01T00:15:00.123456789+01:00"],
            "2026-12-31T23:15:00.123456789",
        ),
        (vec!["--today", "2024-02-29"], "2024-02-29T00:00:00"),
        (
            vec!["--today", "2024-02-29", "--now", "2024-02-29T12:34:56Z"],
            "2024-02-29T12:34:56",
        ),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_probl"))
            .args(["run", file.to_str().unwrap(), "--stats"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(String::from_utf8_lossy(&out.stdout).contains(expected));
        assert!(String::from_utf8_lossy(&out.stdout).contains("100.00%"));
        assert!(String::from_utf8_lossy(&out.stderr).contains(&format!("execution datetime: {expected}")));
    }
    for args in [
        vec!["--now", "2024-02-30T00:00:00"],
        vec!["--now", "2024-02-29"],
        vec!["--today", "2024-02-29", "--now", "2024-03-01T00:00:00"],
    ] {
        assert!(
            !Command::new(env!("CARGO_BIN_EXE_probl"))
                .args(["run", file.to_str().unwrap()])
                .args(args)
                .output()
                .unwrap()
                .status
                .success()
        );
    }
    let out = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(["run", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("100.00%"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pinned_execution_date_is_reported_and_validated() {
    let dir = std::env::temp_dir().join(format!("probl-dates-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("today.probl");
    std::fs::write(&file, "report today\nreport today.add_months(1)").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(["run", file.to_str().unwrap(), "--today", "2024-01-31", "--stats"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("2024-01-31") && text.contains("2024-02-29"), "{text}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("execution date: 2024-01-31"));
    for bad in ["2026-02-29", "2026-1-01", "10000-01-01"] {
        let out = Command::new(env!("CARGO_BIN_EXE_probl"))
            .args(["run", file.to_str().unwrap(), "--today", bad])
            .output()
            .unwrap();
        assert!(!out.status.success());
    }
    // The normal CLI also supplies a snapshot; it does not require --today.
    let out = Command::new(env!("CARGO_BIN_EXE_probl"))
        .args(["run", file.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    std::fs::remove_dir_all(dir).unwrap();
}
