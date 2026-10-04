//! Optional reduction seeds and empty-collection defaults are distinct contracts.
mod common;
use common::{compile_error, exec_raw};
use probl_engine::{ErrorKind, Options, value::Value};
use probl_sema::ir::Mode;

fn options(mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        ..Options::default()
    }
}
fn facts(mode: Mode, source: &str) {
    let out = exec_raw(source, &options(mode)).unwrap_or_else(|e| panic!("{source}: {e:?}"));
    assert!(!out.reports.is_empty());
    for r in out.reports {
        assert_eq!(r.distribution(), vec![(Value::Bool(true), 1.0)], "{source}");
    }
}
fn rejects(mode: Mode, source: &str, message: &str) {
    let e = exec_raw(source, &options(mode)).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
}
macro_rules! both_modes {
    ($($test:item)*) => {
        mod enumerate { use super::*; const MODE:Mode=Mode::Enumerate; $($test)* }
        mod sample { use super::*; const MODE:Mode=Mode::Sample {runs:30,seed:7}; $($test)* }
    };
}
both_modes! {
    #[test]
    fn reduce_folds_left_with_or_without_a_seed() {
        facts(MODE, r#"
            report [10,3,2].reduce((a,b)->a-b)==5
            report [10,3,2].reduce((a,b)->a-b,100)==85
            report [-8,-3,-12].reduce(max)==-3
            report [-8,-3,-12].reduce(max,0)==0
            report reduce(1..4,(a,b)->a+b)==10
            report reduce("a🙂é",(a,b)->a+b)=="a🙂é"
            report [1,2].reduce((s,x)->s+str(x),"n=")=="n=12"
            fn add(a,b){a+b}
            let fold=reduce
            report fold([1,2],add)==3 and fold([1,2],add,10)==13
        "#);
    }

    #[test]
    fn reduce_handles_empty_and_singleton_inputs_explicitly() {
        facts(MODE,r#"
            report [].reduce((a,b)->1/0,42)==42
            report (1..0).reduce(max,-3)==-3
            report "".reduce((a,b)->a+b,"x")=="x"
            report [complex(1,2)].reduce((a,b)->1/0)==complex(1,2)
            report typeof [1.0].reduce(max)=="float"
            report [7].reduce((a,b)->a-b,10)==3
            report pmf([d6].reduce(max),6)==prob(1/6)
        "#);
        for source in ["report [].reduce(max)", "report (1..0).reduce(max)", "report \"\".reduce((a,b)->a+b)", "report [1,2].filter(x->false).reduce(max)"] {
            rejects(MODE,source,"empty collection needs an initial value");
        }
    }

    #[test]
    fn reduce_checks_callbacks_even_without_invoking_them() {
        for xs in ["[]", "[1]", "[1,2]"] {
            for seed in ["", ",0"] {
                rejects(MODE,&format!("report {xs}.reduce(0{seed})"),"function as its second argument");
                rejects(MODE,&format!("report {xs}.reduce(x->x{seed})"),"takes 1 argument");
                rejects(MODE,&format!("report {xs}.reduce(abs{seed})"),"takes 1 argument");
            }
        }
        rejects(MODE,"report [].reduce(0,max)","function as its second argument");
    }

    #[test]
    fn reduction_preserves_recipe_independence_and_collection_lifting() {
        facts(MODE,r#"
            report pmf([d2,d2].reduce(max),1)==25%
            report support(one_of([[],[1,2]]).reduce(max,0))==[0,2]
            report support(one_of([[1],[1,2]]).reduce(max))==[1,2]
            let x ~ d2
            report [x,x].reduce(max)==x
        "#);
        rejects(MODE,"report one_of([[],[1]]).reduce(max)","empty collection");
    }

    #[test]
    fn reduction_callbacks_keep_effect_restrictions() {
        for seed in ["", ",0"] {
            for body in ["{let r ~ d1; a+b}","{observe true; a+b}","if 50% {a+b} else {a-b}"] {
                rejects(MODE,&format!("report [1,2].reduce((a,b)->{body}{seed})"),"can't branch on chances, draw values or observe");
            }
        }
    }

    #[test]
    fn extrema_defaults_are_for_empty_collections_only() {
        facts(MODE,r#"
            report [].maximum(default:42)==42 and [].minimum(default:-42)==-42
            report maximum(1..0,default:"empty")=="empty"
            report "".minimum(default:complex(1))==complex(1)
            report [-8,-3,-12].maximum(default:0)==-3
            report [8,3,12].minimum(default:0)==3
            report "ba".maximum(default:"z")=="b"
            report ["a"].minimum(default:complex(1))=="a"
            report pmf([].maximum(default:d6),6)==prob(1/6)
            let fallback=[].maximum(default:max)
            report fallback(1,3)==3
            report minimum(d6,default:0)==1
            report maximum(uniform(0,1),default:99)==1
        "#);
    }

    #[test]
    fn extrema_defaults_compose_with_comparators_and_aliases() {
        facts(MODE,r#"
            let rows=[{k:2,id:"a"},{k:2,id:"b"}]
            fn compare(a,b){a.k-b.k}
            report rows.maximum(compare,default:{k:10,id:"fallback"}).id=="a"
            report [].minimum(compare,{k:10,id:"fallback"}).id=="fallback"
            report [].maximum((a,b)->1/0,default:4)==4
            let high=maximum
            let low=minimum
            report high([],default:5)==5
            report low(rows,compare,default:{k:10,id:"fallback"}).id=="a"
            report high([],(a,b)->a-b,6)==6
            report [high,low][0]([],default:7)==7
            fn choose(f,xs){f(xs,default:8)}
            report choose(high,[])==8 and choose(low,[3])==3
        "#);
    }

    #[test]
    fn defaults_never_hide_type_callback_or_distribution_errors() {
        for name in ["minimum","maximum"] {
            for xs in ["[complex(1)]", "[d6]"] {
                rejects(MODE,&format!("report {name}({xs},default:0)"), if xs=="[d6]" {"compare"} else {"ordering"});
            }
            rejects(MODE,&format!("report {name}(1,default:0)"),"expects");
            rejects(MODE,&format!("report {name}([],0,default:0)"),"comparator function");
            rejects(MODE,&format!("report {name}([],abs,default:0)"),"takes 1 argument");
            rejects(MODE,&format!("report {name}([1,2],(a,b)->true,default:0)"),"finite int or float");
            rejects(MODE,&format!("report {name}(normal(0,1),default:0)"),"finite support bound");
            rejects(MODE,&format!("report {name}(geometric(50%),default:0)"),"unresolved");
            rejects(MODE,&format!("report {name}(d6,(a,b)->a-b,default:0)"),"comparator");
        }
    }

    #[test]
    fn named_arguments_are_checked_through_function_values() {
        for (source,message) in [
            ("let f=maximum; report f([],fallback:0)","named arguments"),
            ("let f=maximum; report f(default:0)","collection argument"),
            ("let f=maximum; report f([],(a,b)->a-b,0,default:1)","both positionally"),
            ("let f=abs; report f(default:1)","named arguments"),
            ("let f=x->x; report f(default:1)","only builtin"),
        ] {
            rejects(MODE,source,message);
        }
    }

    #[test]
    fn fallback_expressions_are_eager_and_execute_in_source_order() {
        for call in ["maximum", "high"] {
            facts(MODE,&format!(r#"
                let high=maximum
                var n=0
                let result={call}({{n=1; [n]}},default:{{n=2; n}})
                report result==1 and n==2
                let empty={call}([],default:{{n+=1; n}})
                report empty==3 and n==3
            "#));
            rejects(MODE,&format!("let high=maximum; report {call}([1],default:1/0)"),"zero");
        }
    }
}

#[test]
fn malformed_keyword_calls_fail_at_compile_time() {
    for (src, message) in [
        ("report maximum([],default:0,default:1)", "appears twice"),
        ("report maximum([],default:0,(a,b)->a-b)", "positional arguments"),
        ("report maximum([],fallback:0)", "named argument"),
        ("report maximum(default:0)", "collection argument"),
        ("report maximum([],(a,b)->a-b,0,default:1)", "both positionally"),
        ("report abs(default:0)", "named argument"),
        ("let f=maximum; report f([],default:0,default:1)", "appears twice"),
        ("let f=maximum; report f([],default:0,1)", "positional arguments"),
    ] {
        assert!(compile_error(src).contains(message), "{src}");
    }
}

#[test]
fn defaults_and_callbacks_are_visible_to_effect_analysis() {
    for source in [
        "fn f(){[1].maximum(default:{print(\"fallback\"); 0})}; let a=f(); let b=f(); report true",
        "let high=maximum; fn f(){high([1],default:{print(\"fallback\"); 0})}; let a=f(); let b=f(); report true",
        "fn cmp(a,b){print(\"compare\"); a-b}; let r ~ d2; let x=[1,2].maximum(cmp,0); report r",
        "let r ~ d2; let x=[1].maximum(default:{print(\"fallback\"); 0}); report r",
        "let r ~ d2; let x=[1,2].reduce((a,b)->{print(\"combine\"); a+b}); report r",
    ] {
        let (program, diags) = probl_sema::compile(source);
        assert!(program.is_some(), "{source}: {diags:?}");
        let mut prints = Vec::new();
        probl_engine::run(&program.unwrap(), &Options::default(), &mut |s| {
            prints.push(s.to_owned())
        })
        .unwrap();
        assert_eq!(prints.len(), 2, "{source}: {prints:?}");
    }
}
