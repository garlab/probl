//! The environment: a `map[str, str]` constant the host supplies once, like
//! `today`, and that a program can't change.
mod common;
use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use std::sync::Arc;

fn with_env(vars: &[(&str, &str)]) -> Options {
    Options {
        env: Arc::new(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
        ..Options::default()
    }
}

#[test]
fn env_is_the_host_s_variables_as_a_map_of_text() {
    let options = with_env(&[("REGION", "us"), ("RUNS", "10"), ("EMPTY", "")]);
    let out = exec_raw(
        r#"
        report (
            env.get("REGION", "eu") == "us"
            and env.get("MISSING", "eu") == "eu"
            and env["RUNS"] == "10"
            and env["EMPTY"] == ""
            and len(env) == 3
            and "EMPTY" in env and not ("MISSING" in env)
            and typeof env == "map[str, str]"
        )
        report keys(env)
        "#,
        &options,
    )
    .unwrap();
    assert_eq!(out.reports[0].chance(), Some(1.0));
    let keys = Value::list(["EMPTY", "REGION", "RUNS"].map(Value::str).to_vec());
    assert_eq!(out.reports[1].distribution(), vec![(keys, 1.0)]);
    // Without a host's variables, it's empty: the engine never reads the
    // process's environment.
    assert_eq!(chance("report len(env) == 0"), 1.0);
    let e = exec_raw("report env[\"HOME\"]", &Options::default()).unwrap_err();
    assert!(e.message.contains("HOME"), "{}", e.message);
}

#[test]
fn env_is_one_snapshot_across_worlds_samples_calls_and_simulate() {
    let source = r#"
        fn region(): str { env.get("REGION", "eu") }
        let captured = env
        let n ~ d6
        let nested = simulate { let m ~ d6; env.get("REGION", "") + str(m - m) }
        report captured == env and region() == "us" and nested == "us0" and env["N"] == "7"
        report env.get("REGION", "eu")
    "#;
    let program = probl_sema::compile(source).0.unwrap();
    for threads in [1, 4] {
        for mode in [None, Some(probl_sema::ir::Mode::Sample { runs: 2500, seed: 7 })] {
            let options = Options {
                mode,
                limits: Limits {
                    max_threads: threads,
                    ..Limits::default()
                },
                ..with_env(&[("REGION", "us"), ("N", "7")])
            };
            let out = probl_engine::run(&program, &options, &mut |_| {}).unwrap();
            assert_eq!(out.reports[0].chance(), Some(1.0));
            assert_eq!(out.reports[1].distribution(), vec![(Value::str("us"), 1.0)]);
        }
    }
}

#[test]
fn env_is_a_constant_a_program_can_t_change() {
    assert!(compile_error("env = [\"A\": \"b\"]").contains("can't assign to `env`: it's a constant"));
    assert!(compile_error("env[\"A\"] = \"b\"").contains("it's a constant"));
    assert!(compile_error("env.insert(\"A\", \"b\")").contains("it's a constant"));
    assert!(compile_error("report env()").contains("constant, not a function"));
    // A copy can change; `env` doesn't.
    let options = with_env(&[("A", "a")]);
    let out = exec_raw(
        "var copy = env\ncopy[\"A\"] = \"b\"\ncopy.insert(\"C\", \"c\")\nreport copy[\"A\"] == \"b\" and env[\"A\"] == \"a\" and len(env) == 1",
        &options,
    )
    .unwrap();
    assert_eq!(out.reports[0].chance(), Some(1.0));
    // A program's own name hides it.
    assert_eq!(
        chance("let env = [\"NAME\": \"X\"]\nreport env.get(\"NAME\", \"Y\") == \"X\""),
        1.0
    );
    assert_eq!(chance("fn env() { 7 }\nreport env() == 7"), 1.0);
}

#[test]
fn the_run_s_limits_apply_to_the_environment() {
    let options = Options {
        limits: Limits {
            max_string_bytes: 4,
            ..Limits::default()
        },
        ..with_env(&[("A", "too long")])
    };
    let e = exec_raw("report len(env)", &options).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit, "{e:?}");
    assert!(e.message.contains("`A`"), "{}", e.message);
    // A program that doesn't use `env` isn't affected.
    assert!(exec_raw("report 1", &options).is_ok());
}
