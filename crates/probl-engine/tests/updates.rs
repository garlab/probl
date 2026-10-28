//! `push`, `pop`, `insert` and `remove` change a collection in place when
//! nothing else holds it (docs/design/collection-values.md). Whether they
//! do or copy it, the result is the same as an assignment's.
mod common;
use common::*;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Options};
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
fn a_change_is_never_seen_through_another_name() {
    for mode in MODES {
        facts(
            mode,
            r#"
            let a = ["A"]
            var b = [a]
            b[0].push("B")
            b[0].push("C")
            report a == ["A"] and b == [["A", "B", "C"]]
            var c = b
            c[0].pop()
            c.push(["D"])
            report b == [["A", "B", "C"]] and c == [["A", "B"], ["D"]]
            var m = ["k": [1]]
            let n = m
            m["k"].insert(0, 0)
            m.insert("j", [])
            m.remove("k")
            report n == ["k": [1]] and m == ["j": []]
            var xs = [1, 2, 3]
            let ys = xs
            xs.remove(0)
            xs.insert(1, 9)
            let top = xs.pop()
            report ys == [1, 2, 3] and top == 3 and xs == [2, 9]
            fn grow(zs) { var copy = zs; copy.push(0); copy }
            report grow(xs) == [2, 9, 0] and xs == [2, 9]
        "#,
        );
    }
}

#[test]
fn worlds_change_their_own_copies() {
    let src = "var xs = [1]\nlet d ~ d2\nif d == 2 { xs.push(2) }\nxs.push(3)\nreport xs";
    assert_eq!(
        distribution(src),
        vec![
            (
                Value::list(vec![Value::Int(1.into()), Value::Int(2.into()), Value::Int(3.into())]),
                0.5
            ),
            (Value::list(vec![Value::Int(1.into()), Value::Int(3.into())]), 0.5),
        ]
    );
    // A loop that pushes in every world, as a hand of cards does.
    let src = "var hand = []\nrepeat 3 { let card ~ d2\nhand.push(card) }\nreport len(hand) == 3 and hand.count(c -> c == 1) == 1";
    close(chance(src), 3.0 / 8.0);
}

#[test]
fn a_caught_fault_sees_the_collection_as_its_statement_began() {
    for mode in MODES {
        facts(
            mode,
            r#"
            var xs = [1, 2]
            let r = try { xs.insert(9, 0); 1 } catch IndexOutOfBounds { 0 }
            let s = try { xs.remove(5); 1 } catch IndexOutOfBounds { 0 }
            var empty = []
            let t = try { empty.pop() } catch EmptyCollection { -1 }
            report r == 0 and s == 0 and t == -1 and xs == [1, 2] and empty == []
            var m = [1: "a"]
            let u = try { m.insert(2, "b"); m[3] } catch MissingKey { "none" }
            report u == "none" and m == [1: "a", 2: "b"]
        "#,
        );
    }
}

#[test]
fn declared_types_check_what_goes_in() {
    for mode in MODES {
        facts(
            mode,
            r#"
            var xs: list[float] = []
            xs.push(2)
            xs.insert(0, 1)
            report xs == [1, 2] and typeof xs == "list[float]"
            var ps: list[list[prob]] = [[]]
            ps[0].push(30%)
            report typeof ps == "list[list[prob]]"
            var m: map[str, float] = [:]
            m.insert("a", 1%)
            report typeof m["a"] == "float"
            type Hand = { cards: list[float] }
            var h = Hand { cards: [] }
            h.cards.push(1)
            report h.cards == [1] and typeof h.cards == "list[float]"
        "#,
        );
    }
    for (src, message) in [
        (
            "var xs: list[int] = []\nxs.push(\"a\")",
            "`xs` should be a list[int], but it's a list[str]",
        ),
        (
            "var xs: list[list[int]] = [[]]\nlet h = 2.0\nxs[0].push(h)",
            "`xs` should be a list[list[int]], but it's a list[list[float]]",
        ),
        (
            "var xs: list[list[int]] = [[]]\nxs[0].push(\"a\")",
            "`xs` should be a list[list[int]], but it's a list[list[str]]",
        ),
        (
            "var ps: list[prob] = []\nlet h = 1.5\nps.push(h)",
            "prob needs a finite number between 0 and 1",
        ),
        (
            "type Hand = { cards: list[int] }\nvar h = Hand { cards: [] }\nh.cards.push(\"a\")",
            "should be",
        ),
    ] {
        let e = exec_raw(src, &Options::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Language, "{src}: {e:?}");
        assert!(e.message.contains(message), "{src}: {e:?}");
    }
}

#[test]
fn methods_still_lift_over_distributions() {
    let src = "var xs = [1]\nxs.push(d2)\nreport xs";
    assert_eq!(
        distribution(src),
        vec![
            (Value::list(vec![Value::Int(1.into()), Value::Int(1.into())]), 0.5),
            (Value::list(vec![Value::Int(1.into()), Value::Int(2.into())]), 0.5),
        ]
    );
    // A range isn't a list to change, as before.
    assert!(error("var r = 1..3\nr.push(4)").contains("`push` needs a list, found a range"));
}

#[test]
fn limits_still_apply() {
    let options = Options {
        limits: probl_engine::Limits {
            max_collection: 3,
            ..probl_engine::Limits::default()
        },
        ..Options::default()
    };
    let e = exec_raw("var xs = [1, 2, 3]\nxs.push(4)", &options).unwrap_err();
    assert!(e.message.contains("over the limit"), "{e:?}");
}
