mod common;

use common::*;
use probl_engine::value::Value;

#[test]
fn taking_weights_by_count_and_removes_exactly_one_copy() {
    let src = "var deck = bag([1: 2, 2: 1])\n\
               let first = deck.take()\n\
               let second = deck.take()\n\
               report first == 1 and second == 1\n\
               report len(deck)";
    let out = outcome(src);
    close(out.reports[0].chance().unwrap(), 1.0 / 3.0);
    assert_eq!(out.reports[1].distribution(), vec![(Value::Int(1.into()), 1.0)]);

    for mode in ["enumerate", "sample(runs: 1000, seed: 7)"] {
        close(
            chance(&format!(
                "@mode {mode}\nvar deck = bag([1, 2])\n\
                 let first = deck.take()\nlet alias = first\nlet second = deck.take()\n\
                 report first == alias and first != second and len(deck) == 0"
            )),
            1.0,
        );
    }
}

#[test]
fn taking_is_an_expression_with_ordered_effects() {
    close(
        chance(
            "var deck = bag([1, 2])\n\
             let xs = [len(deck), deck.take(), len(deck), deck.take(), len(deck)]\n\
             report xs[0] == 2 and xs[2] == 1 and xs[4] == 0 and xs[1] != xs[3]",
        ),
        1.0,
    );
    close(
        chance(
            "var state = { decks: [bag([1]), bag([2])] }\nvar calls = 0\n\
             let item = state.decks[{ calls += 1; 0 }].take()\n\
             report item == 1 and calls == 1 and len(state.decks[0]) == 0 and len(state.decks[1]) == 1",
        ),
        1.0,
    );
    close(
        chance(
            "var deck = bag([[1, 2]])\nlet [a, b] = deck.take()\n\
             report a == 1 and b == 2 and len(deck) == 0",
        ),
        1.0,
    );
    close(
        chance(
            "var deck = bag([3])\nvar xs = [0]\nxs[0] += deck.take()\n\
             report xs[0] == 3 and len(deck) == 0",
        ),
        1.0,
    );
}

#[test]
fn unused_results_still_consume_but_skipped_expressions_do_not() {
    close(
        chance(
            "var deck = bag([true, false])\n\
             let a = false and deck.take()\nlet b = true or deck.take()\n\
             let c = if false { deck.take() } else { true }\n\
             deck.take()\nreport not a and b and c and len(deck) == 1",
        ),
        1.0,
    );
}

#[test]
fn probability_and_distribution_items_are_returned_unchanged() {
    for mode in ["enumerate", "sample(runs: 100, seed: 7)"] {
        close(
            chance(&format!(
                "@mode {mode}\nvar rates = bag([prob(25%)])\nvar recipes = bag([d2])\n\
                 let p = rates.take()\nlet d = recipes.take()\nlet event = ~p\nlet face = ~d\n\
                 report typeof p == \"prob\" and typeof d == \"dist[int]\" and\n\
                 typeof event == \"bool\" and typeof face == \"int\" and len(rates) == 0 and len(recipes) == 0"
            )),
            1.0,
        );
    }
    assert_eq!(
        distribution("var rates = bag([prob(25%), prob(75%)])\nlet p = rates.take()\nreport p"),
        vec![(Value::Prob(0.25), 0.5), (Value::Prob(0.75), 0.5)]
    );
    close(
        chance(
            "var recipes = bag([d2, d4])\nlet d: dist[int] = recipes.take()\n\
             report typeof d == \"dist[int]\" and len(recipes) == 1 and (mean(d) == 1.5 or mean(d) == 2.5)",
        ),
        1.0,
    );
    // All draw forms act on the returned item, rather than merely extracting it.
    for binding in [
        "let x = ~items.take()",
        "let x ~ items.take()",
        "var x = false; x = ~items.take()",
        "var x = false; x ~ items.take()",
    ] {
        close(
            chance(&format!("var items = bag([prob(25%)])\n{binding}\nreport x")),
            0.25,
        );
        close(mean(&format!("var items = bag([d2])\n{binding}\nreport x")), 1.5);
    }
}

#[test]
fn observation_consumes_the_item_before_conditioning() {
    for observation in ["observe cards.take()", "observe ~cards.take()"] {
        let out = outcome(&format!(
            "var cards = bag([true, false])\n{observation}\n\
             report len(cards) == 1 and cards.take() == false"
        ));
        close(out.evidence.unwrap().to_f64(), 0.5);
        close(out.reports[0].chance().unwrap(), 1.0);
    }
    for observation in ["score rates.take()", "observe ~rates.take()"] {
        let out = outcome(&format!(
            "var rates = bag([prob(25%), prob(75%)])\n{observation}\nreport rates.take()"
        ));
        close(out.evidence.unwrap().to_f64(), 0.5);
        let remaining = out.reports[0].distribution();
        assert_eq!(remaining, vec![(Value::Prob(0.25), 0.75), (Value::Prob(0.75), 0.25)]);
    }
    error("var rates = bag([prob(50%)])\nobserve rates.take()\nreport true");
}

#[test]
fn functions_and_callbacks_keep_their_effect_boundaries() {
    close(
        chance(
            "fn pair() { var deck = bag([1, 2]); [deck.take(), deck.take()] }\n\
             let xs = pair()\nreport xs[0] != xs[1]",
        ),
        1.0,
    );
    for mode in ["enumerate", "sample(runs: 100, seed: 7)"] {
        for source in [
            "report [1].map(x -> { var deck = bag([1]); deck.take() })",
            "fn pick() { var deck = bag([1]); deck.take() }\nlet cached = pick()\nreport [cached].map(x -> pick())",
        ] {
            let e = error(&format!("@mode {mode}\n{source}"));
            assert!(e.contains("can't branch on chances, draw values or observe"), "{e}");
        }
        close(
            chance(&format!(
                "@mode {mode}\nreport [1].map(x -> simulate {{ var deck = bag([true]); deck.take() }})[0]"
            )),
            1.0,
        );
    }
    // User-defined methods named `take` retain normal call semantics.
    close(mean("fn take(x) { x + 1 }\nreport (41).take()"), 42.0);
}

#[test]
fn taking_requires_a_mutable_nonempty_bag_and_checks_result_types() {
    for source in [
        "let deck = bag([1]); let x = deck.take()",
        "var deck = bag([1]); fn f() { deck.take() }; let x = f()",
        "let x = bag([1]).take()",
        "var deck = bag([1]); let x = deck.take(1)",
        "var deck = bag([1]); let x = deck.take(n: 1)",
        "var deck = bag([1]); let x = take(deck)",
    ] {
        compile_error(source);
    }
    for source in [
        "var deck = bag([]); let x = deck.take()",
        "var deck = bag([1]); deck.take(); deck.take()",
        "var deck = [1]; let x = deck.take()",
        "var deck = bag([1]); let x: str = deck.take()",
        "var deck = bag([prob(25%)]); let x: bool = deck.take()",
    ] {
        error(source);
    }
    close(
        chance("var deck: bag[int] = bag([1]); let x: int = deck.take(); report x == 1 and len(deck) == 0"),
        1.0,
    );
}
