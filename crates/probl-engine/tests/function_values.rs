//! Named and builtin functions share the existing closure/callback contracts.
mod common;
use common::{compile_error, exec_raw};
use probl_engine::{ErrorKind, Options, value::Value};
use probl_sema::ir::Mode;
const MODES: [Mode; 2] = [Mode::Enumerate, Mode::Sample { runs: 30, seed: 7 }];
fn facts(mode: Mode, src: &str) {
    let out = exec_raw(
        src,
        &Options {
            mode: Some(mode),
            ..Options::default()
        },
    )
    .unwrap_or_else(|e| panic!("{src}: {e:?}"));
    for r in out.reports {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)], "{src}");
    }
}

#[test]
fn named_functions_capture_values_and_preserve_declared_boundaries() {
    for mode in MODES {
        facts(
            mode,
            r#"
            var factor=2
            fn scale(x:int)->int {x*factor}
            let f=scale
            factor=3
            report f(4.0)==8 and scale(4)==12
            report [1,2].map(f)==[2,4]
            fn compare(a,b) {a-b}
            report [3,1,2].sort(compare)==[1,2,3]
            report [3,1,2].maximum(compare)==3
            fn identity(x:prob)->prob {x}
            let g=identity
            report typeof g=="fn" and typeof g(50%)=="prob"
            fn outer(x) { x + factor }
            fn pick() { outer }
            let h=pick()
            report h(2)==5
        "#,
        );
    }
}

#[test]
fn builtin_values_support_optional_and_variadic_arities_and_higher_order_calls() {
    for mode in MODES {
        facts(
            mode,
            r#"
            let f=max
            report f(1,5,3)==5 and pmf(f(d2,1),1)==50%
            report [1,2,3].reduce(0,max)==3
            report [-2,3,-4].map(abs)==[2,3,4]
            let r=round
            report r(1.5)==2 and r(1.234,2)==1.23
            let dt=date
            report dt("2026-01-02")==dt(2026,1,2)
            let b=bernoulli
            report pmf(b(30%),true)==30%
            let m=maximum
            report m(d6)==6 and m([complex(1),complex(2)],(a,b)->abs(a)-abs(b))==complex(2)
            let traversal=map
            report traversal([-1,2],abs)==[1,2]
            report typeof abs=="fn" and typeof (x->x)=="fn"
            report [max,min][0](1,2)==2
            let n=max
            report [max:n].get(max)==max
        "#,
        );
    }
}

#[test]
fn function_values_validate_runtime_arity_and_types() {
    for mode in MODES {
        for src in [
            "let f=max; report f(1)",
            "let f=round; report f(1,2,3)",
            "let f=date; report f(2026,1)",
            "fn foo(x){x}; let f=foo; report f(1,2)",
            "report [1,2].sort(abs)",
        ] {
            let e = exec_raw(
                src,
                &Options {
                    mode: Some(mode.clone()),
                    ..Options::default()
                },
            )
            .unwrap_err();
            assert_eq!(e.kind, ErrorKind::Language, "{src}: {e:?}");
            assert!(e.message.contains("takes"), "{src}: {e:?}");
        }
        for src in [
            "fn f(x:int){x}; let g=f; report g(1.4)",
            "let f=bernoulli; report f(1.4)",
        ] {
            assert_eq!(
                exec_raw(
                    src,
                    &Options {
                        mode: Some(mode.clone()),
                        ..Options::default()
                    }
                )
                .unwrap_err()
                .kind,
                ErrorKind::Language
            );
        }
    }
}

#[test]
fn named_callbacks_keep_effect_restrictions_and_local_simulation() {
    for mode in MODES {
        for src in [
            "fn cmp(a,b){let x ~ d1; a-b}; report [1,2].maximum(cmp)",
            "fn cmp(a,b){observe true; a-b}; report [1,2].highest(1,cmp)",
            "fn f(x){let y ~ d1; y}; report [1].map(f)",
        ] {
            let e = exec_raw(
                src,
                &Options {
                    mode: Some(mode.clone()),
                    ..Options::default()
                },
            )
            .unwrap_err();
            assert!(
                e.message.contains("can't branch on chances, draw values or observe"),
                "{e:?}"
            );
        }
        facts(
            mode,
            "fn f(x){simulate {let r ~ d2; x+r}}; report maximum([1].map(f)[0])==3",
        );
    }
}

#[test]
fn observation_effects_through_named_references_are_checked() {
    assert!(
        compile_error("fn evidence(){observe true; 1}; let f=evidence; report true; let x=f()")
            .contains("after a `report`")
    );
}

#[test]
fn direct_function_values_still_split_worlds() {
    let src = "fn die(){let x ~ d2; x}; let f=die; report f()";
    let out = exec_raw(src, &Options::default()).unwrap();
    assert_eq!(
        out.reports[0].distribution(),
        vec![(Value::Int(1.into()), 0.5), (Value::Int(2.into()), 0.5)]
    );
}

#[test]
fn callback_print_effects_prevent_caching_and_unsafe_draw_scheduling() {
    for src in [
        "fn cmp(a,b){print(\"compare\"); a-b}; fn choose(){[1,2].maximum(cmp)}; let a=choose(); let b=choose(); report a==b",
        "fn cmp(a,b){print(\"compare\"); a-b}; let r ~ d2; let x=[1,2].minimum(cmp); report r",
        "let p=print; fn f(){[1].map(p)}; let a=f(); let b=f(); report true",
        "let r ~ d2; let x=[1].map(print); report r",
        "let p=print; p(1,2,3); p()",
        "fn cmp(a,b){print(\"compare\"); a-b}; let high=highest; fn f(){high([1,2],1,cmp)}; let a=f(); let b=f(); report true",
    ] {
        let (program, diags) = probl_sema::compile(src);
        assert!(program.is_some(), "{src}: {diags:?}");
        let mut prints = Vec::new();
        probl_engine::run(&program.unwrap(), &Options::default(), &mut |s| {
            prints.push(s.to_owned())
        })
        .unwrap();
        assert_eq!(prints.len(), 2, "{src}: {prints:?}");
    }
}
