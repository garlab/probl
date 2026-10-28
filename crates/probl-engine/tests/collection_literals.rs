//! A collection literal takes its kind from the declared type where the
//! compiler knows it: `[…]` is a bag where a bag is declared, and `[k: v, …]`
//! a bag of counts. An existing collection never changes kind.
mod common;

use common::*;
use probl_engine::{ErrorKind, Options};
use probl_sema::ir::Mode;

fn options(mode: Mode) -> Options {
    Options {
        mode: Some(mode),
        ..Options::default()
    }
}

fn facts(mode: Mode, source: &str) {
    let out = exec_raw(source, &options(mode)).unwrap_or_else(|e| panic!("{source}\n{e:?}"));
    assert!(!out.reports.is_empty());
    for report in out.reports {
        assert_eq!(
            report.distribution(),
            vec![(probl_engine::value::Value::Bool(true), 1.0)],
            "{source}"
        );
    }
}

fn rejects(mode: Mode, source: &str, message: &str, help: &str) {
    let e = exec_raw(source, &options(mode)).expect_err(source);
    assert_eq!(e.kind, ErrorKind::Language, "{source}: {e:?}");
    assert!(e.message.contains(message), "{source}: {e:?}");
    assert!(e.help.as_deref().is_some_and(|h| h.contains(help)), "{source}: {e:?}");
}

macro_rules! both_modes {
    ($($test:item)*) => {
        mod enumerate {
            use super::*;
            const MODE: Mode = Mode::Enumerate;
            $($test)*
        }
        mod sample {
            use super::*;
            const MODE: Mode = Mode::Sample { runs: 20, seed: 7 };
            $($test)*
        }
    };
}

both_modes! {
    #[test]
    fn a_literal_is_a_bag_wherever_a_bag_is_declared() {
        facts(
            MODE,
            r#"
            let b: bag[int] = [1, 1, 2]
            report typeof b == "bag[int]" and len(b) == 3 and b.get(1) == 2
            let deck: bag[str] = ["A": 4, "K": 4]
            report typeof deck == "bag[str]" and len(deck) == 8 and deck.get("K") == 4
            let empty: bag[int] = []
            let none: bag[int] = [:]
            report len(empty) == 0 and len(none) == 0 and typeof empty == "bag[unknown]"
            var v: bag[int] = []
            v = [3, 3]
            report v.get(3) == 2
            let chosen: bag[int] = if 50% { [1, 1] } else { [2] }
            let matched: bag[int] = match 1 { 1 => [1, 1], _ => [2] }
            let blocked: bag[int] = { [1, 2] }
            report typeof chosen == "bag[int]" and typeof matched == "bag[int]" and typeof blocked == "bag[int]"
            let nested: list[bag[int]] = [[1, 1], [2]]
            report typeof nested == "list[bag[int]]" and nested[0].get(1) == 2
            let keyed: map[bag[int], str] = [[1, 1]: "pair"]
            report typeof keyed == "map[bag[int], str]"
            let floats: bag[float] = [1, 1.0, 2]
            report typeof floats == "bag[float]" and floats.get(1) == 2
            fn size(b: bag[int]): int { len(b) }
            fn made(): bag[int] { [5, 5] }
            report size([1, 1, 2]) == 3 and typeof made() == "bag[int]"
            type Hand = { cards: bag[int] }
            let h = Hand { cards: [1, 2] }
            report typeof h.cards == "bag[int]"
        "#,
        );
    }

    #[test]
    fn an_existing_list_never_becomes_a_bag() {
        let help = "a list never becomes a bag";
        rejects(MODE, "let xs = [1, 1, 2]\nlet b: bag[int] = xs", "`b` should be a bag[int], but it's a list[int]", help);
        // Through a function value, the compiler doesn't know the type, so
        // the literal is a list, which the call rejects.
        rejects(
            MODE,
            "fn f(b: bag[int]) { len(b) }\nlet g = f\nreport g([1, 1, 2])",
            "`b` should be a bag[int], but it's a list[int]",
            help,
        );
        // Nor does it in a `with` on a record of unknown type, written in
        // place or in a branch.
        for value in ["[1, 2]", "if true { [1, 2] } else { [3] }"] {
            rejects(
                MODE,
                &format!(
                    "type Hand = {{ cards: bag[int] }}\nfn redeal(r) {{ r with {{ cards: {value} }} }}\nreport redeal(Hand {{ cards: [3] }})"
                ),
                "field `cards` should be bag[int]",
                help,
            );
        }
    }

    #[test]
    fn a_bag_of_counts_takes_whole_counts() {
        facts(MODE, "let b: bag[str] = [\"a\": 0, \"b\": 2]\nreport len(b) == 2 and b.get(\"a\") == 0");
        let e = exec_raw("let b: bag[str] = [\"a\": -1]", &options(MODE)).unwrap_err();
        assert!(e.message.contains("bag counts"), "{e:?}");
    }
}

#[test]
fn a_count_that_isn_t_an_int_fails_before_the_program_runs() {
    assert!(compile_error("let b: bag[str] = [\"a\": 2.0]").contains("expected an int, found a float"));
}
