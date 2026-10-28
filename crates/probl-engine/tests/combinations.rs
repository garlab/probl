mod common;
use common::*;
use probl_engine::{ErrorKind, Limits, Options};

#[test]
fn combinations_follow_input_positions_and_preserve_equal_values() {
    for mode in ["enumerate", "sample(runs: 50, seed: 7)"] {
        for source in [
            "report [3,1,2].combinations(2)==[[3,1],[3,2],[1,2]]",
            "report combinations([1,1,2],2)==[[1,1],[1,2],[1,2]]",
            "report (1..4).combinations(3)==[[1,2,3],[1,2,4],[1,3,4],[2,3,4]]",
            "report \"a🙂é\".combinations(2)==[[\"a\",\"🙂\"],[\"a\",\"é\"],[\"🙂\",\"é\"]]",
            "report [1,2].combinations(0)==[[]] and [].combinations(0)==[[]] and [].combinations(1)==[]",
            "report [1,2].combinations(3)==[] and [1,2].combinations(1)==[[1],[2]] and [1,2].combinations(2)==[[1,2]]",
            "let f=combinations; report f([complex(1),complex(2)],2)==[[complex(1),complex(2)]]",
            "let xs=[d2,prob(25%)]; let ys=xs.combinations(1); report typeof ys[0][0]==\"dist[int]\" and typeof ys[1][0]==\"prob\"",
            "report [1,2,3,4].combinations(2).count(pair -> sum(pair)==5)==2",
        ] {
            close(chance(&format!("@mode {mode}\n{source}")), 1.0);
        }
    }
    // The lifted argument is a distribution of lists, not draws of their elements.
    close(chance("report combinations(one_of([[1,2],[2,3]]),2)==[[1,2]]"), 0.5);
    close(chance("report len(combinations([1,2,3],d2))==3"), 1.0);
    close(
        chance("let x ~ uniform(0,1); let pairs=[x,x+1].combinations(2); report pairs[0][1]>1.5"),
        0.5,
    );
}

#[test]
fn counts_match_binomial_coefficients_and_inputs_are_unchanged() {
    for n in 0..=8 {
        for k in 0..=n + 1 {
            close(
                chance(&format!(
                    "let xs=0..<{n}; let groups=xs.combinations({k}); report len(groups)==choose({n},{k}) and xs==0..<{n} and groups.count(g -> len(g)=={k})==len(groups)"
                )),
                1.0,
            );
        }
    }
}

#[test]
fn invalid_inputs_and_counts_are_rejected() {
    for source in [
        "report combinations([1,2],-1)",
        "report combinations([1,2],1.5)",
        "report combinations([1,2],true)",
        "report combinations([1,2],\"1\")",
        "report combinations([1:2],1)",
        "report combinations(bag([1,2]),1)",
        "report combinations(42,0)",
    ] {
        error(source);
    }
    for source in [
        "report combinations([1])",
        "report [1].combinations()",
        "report [1].combinations(1,2)",
    ] {
        compile_error(source);
    }
    close(
        chance("report try { [1].combinations(-1)==[] } catch DomainError { true }"),
        1.0,
    );
}

#[test]
fn limits_are_checked_before_building_the_result() {
    let options = Options {
        limits: Limits {
            max_collection: 10,
            ..Limits::default()
        },
        ..Options::default()
    };
    for source in [
        "report (1..6).combinations(3)",
        "report (1..1000).combinations(1)",
        "report (1..10^100).combinations(2)",
    ] {
        assert_eq!(exec_raw(source, &options).unwrap_err().kind, ErrorKind::Limit);
    }
    for source in [
        "report (0..10^100).combinations(0)==[[]]",
        "report (0..10^100).combinations(10^100+2)==[]",
    ] {
        assert!(exec_raw(source, &options).is_ok());
    }
    let options = Options {
        limits: Limits {
            max_work: 100,
            ..Limits::default()
        },
        ..Options::default()
    };
    assert_eq!(
        exec_raw("report (1..20).combinations(10)", &options).unwrap_err().kind,
        ErrorKind::Limit
    );
}

#[test]
fn set_example_rules_match_exact_counts() {
    let source = include_str!("../../../examples/20_set.probl");
    let cards = source.lines().find(|line| line.starts_with("let cards =")).unwrap();
    let rules = &source[source.find("fn is_set(").unwrap()..];
    // Every pair of distinct SET cards determines exactly one third card;
    // each set has three pairs, so the full deck has 81*80/6 = 1080 sets.
    close(
        chance(&format!(
            "{cards}\n{rules}\nreport cards.combinations(3).count(is_set)==1080"
        )),
        1.0,
    );
    // In the nine-card subdeck varying two attributes, there are 12 sets
    // among C(9,3)=84 hands. Check the whole pipeline by enumeration.
    close(
        chance(&format!(
            "{cards}\n{rules}\nvar deck=bag(cards.slice(0,9)); report is_set(deck.take(3))"
        )),
        1.0 / 7.0,
    );
}

#[test]
fn set_any_and_count_give_identical_seeded_results() {
    let source = include_str!("../../../examples/20_set.probl").replace("10_000", "1_000");
    let counted = source.replace(
        "not hand.combinations(3).any(is_set)",
        "hand.combinations(3).count(is_set) == 0",
    );
    assert_ne!(source, counted);
    let fast = outcome(&source);
    let full = outcome(&counted);
    assert_eq!(fast.reports[0].distribution(), full.reports[0].distribution());
    assert!(fast.stats.calls < full.stats.calls);
}
