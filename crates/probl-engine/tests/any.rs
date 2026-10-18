mod common;
use common::*;
use probl_engine::{ErrorKind, Limits, Options};

#[test]
fn any_checks_sequences_in_order_and_stops_at_the_first_match() {
    for mode in ["enumerate", "sample(runs: 50, seed: 7)"] {
        for source in [
            "report [1,2,3].any(x -> x==2) and not [1,2,3].any(x -> x==4)",
            "report (1..4).any(x -> x==4) and not (1..0).any(x -> true)",
            "report \"a🙂é\".any(c -> c==\"🙂\") and not \"\".any(c -> true)",
            "report not [].any(x -> 1/0)",
            "report [0,1].any(x -> if x==0 {true} else {1/0>0})",
            "report [true,d2].any(x -> x)",
            "fn hit(x) { x==3 }; let f=any; report f([1,3],hit)",
            "report [1].any(x -> P(simulate { 1 } == 1) == 1)",
        ] {
            close(chance(&format!("@mode {mode}\n{source}")), 1.0);
        }
    }
    close(chance("report any(one_of([[1,2],[3,4]]), x -> x==1)"), 0.5);
    close(chance("let x ~ uniform(0,1); report [x].any(v -> true)"), 1.0);
}

#[test]
fn callback_validation_and_effects_apply_only_to_visited_elements() {
    for mode in ["enumerate", "sample(runs: 50, seed: 7)"] {
        for source in [
            "report [].any(42)",
            "report [].any((x,y) -> true)",
            "report [1].any(x -> 1)",
            "report [1].any(x -> prob(1))",
            "report [1].any(x -> d2==1)",
            "report [1:2].any(x -> true)",
        ] {
            error(&format!("@mode {mode}\n{source}"));
        }
        for body in [
            "~bernoulli(1)",
            "{ observe true; true }",
            "{ score 50%; true }",
            "if 50% { true } else { false }",
            "{ var b=bag([true]); b.take() }",
        ] {
            let source = format!("@mode {mode}\nfn test(x) {{ if x==0 {{ true }} else {{ {body} }} }}\n");
            close(chance(&(source.clone() + "report [0,1].any(test)")), 1.0);
            assert!(
                error(&(source + "report [1].any(test)")).contains("can't branch on chances, draw values or observe")
            );
        }
        close(
            chance(&format!(
                "@mode {mode}\nreport try {{ [1].any(x -> 1/0>0) }} catch DivisionByZero {{ true }}"
            )),
            1.0,
        );
    }
    for source in ["report any([1])", "report [1].any()", "report any([1],x -> true,0)"] {
        compile_error(source);
    }
}

#[test]
fn printing_callbacks_visit_only_the_prefix_and_are_never_cached() {
    for search in ["any", "search"] {
        let source = format!(
            "let search=any\nfn test(x) {{ print(x); x==2 }}\nfn search_again() {{ {search}([1,2,3],test) }}\nrepeat 2 {{ let found=search_again() }}"
        );
        for mode in ["enumerate", "sample(runs: 1, seed: 7)"] {
            let source = format!("@mode {mode}\n{source}");
            let (program, diagnostics) = probl_sema::compile(&source);
            assert!(diagnostics.is_empty());
            let mut lines = Vec::new();
            probl_engine::run(&program.unwrap(), &Options::default(), &mut |s| {
                lines.push(s.to_string())
            })
            .unwrap();
            assert_eq!(lines, ["1", "2", "1", "2"]);
        }
    }
}

#[test]
fn any_respects_work_and_materialization_limits() {
    let options = Options {
        limits: Limits {
            max_collection: 10,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        exec_raw("report (1..1000).any(x -> true)", &options).unwrap_err().kind,
        ErrorKind::Limit
    );
    let options = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        exec_raw("report (1..100).any(x -> false)", &options).unwrap_err().kind,
        ErrorKind::Limit
    );
}
