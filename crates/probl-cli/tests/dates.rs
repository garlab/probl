//! The executable captures a date once and accepts an explicit replay date.
use std::process::Command;

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
