//! Comparing collections and copying one to change it count as work, so the
//! work limit bounds them (docs/semantics.md, section 11). Each control does
//! the same steps without the comparisons or copies, and stays under the
//! limit its variant goes over.
mod common;
use common::exec_raw;
use probl_engine::{ErrorKind, Limits, Options};

fn limited() -> Options {
    Options {
        limits: Limits {
            max_work: 1_000_000,
            ..Limits::default()
        },
        ..Options::default()
    }
}

fn costs(control: &str, variant: &str) {
    if let Err(e) = exec_raw(control, &limited()) {
        panic!("{control}: {e:?}");
    }
    match exec_raw(variant, &limited()) {
        Err(e) => assert_eq!(e.kind, ErrorKind::Limit, "{variant}: {e:?}"),
        Ok(_) => panic!("{variant}: finished within the limit"),
    }
}

const LISTS: &str = "let xs = (1..2000).map(x -> x)\n";

#[test]
fn comparisons_count_each_element_compared() {
    // A list is equal to itself without comparing its elements, and lists
    // that differ in their first element are ordered after comparing it.
    for (op, control) in [("==", "xs"), ("<=", "[2]")] {
        costs(
            &format!("{LISTS}let ys = {control}\nvar c = 0\nrepeat 1000 {{ if xs {op} ys {{ c += 1 }} }}\nreport c"),
            &format!(
                "{LISTS}let ys = (1..2000).map(x -> x)\nvar c = 0\nrepeat 1000 {{ if xs {op} ys {{ c += 1 }} }}\nreport c"
            ),
        );
    }
    costs(
        &format!("{LISTS}var c = 0\nrepeat 1000 {{ if xs.contains(1) {{ c += 1 }} }}\nreport c"),
        &format!("{LISTS}var c = 0\nrepeat 1000 {{ if xs.contains(2000) {{ c += 1 }} }}\nreport c"),
    );
    costs(
        "let a = [1: (1..2000).map(x -> x)]\nlet b = a\nvar c = 0\nrepeat 1000 { if a == b { c += 1 } }\nreport c",
        "let a = [1: (1..2000).map(x -> x)]\nlet b = [1: (1..2000).map(x -> x)]\nvar c = 0\nrepeat 1000 { if a == b { c += 1 } }\nreport c",
    );
}

#[test]
fn copies_to_change_a_shared_collection_count_each_element() {
    // Changed while another variable holds it, the list is copied each time.
    // Each round changes it, so that the loop runs every round, and reads
    // the other variable after, so that it still holds the list then.
    costs(
        &format!(
            "{LISTS}var ys = xs\nvar zs = ys\nvar i = 0\nvar t = 0\nrepeat 1000 {{ i += 1\nys[0] = i\nt += zs[0] }}\nreport t"
        ),
        &format!(
            "{LISTS}var ys = xs\nvar zs = ys\nvar i = 0\nvar t = 0\nrepeat 1000 {{ i += 1\nys[0] = i\nt += zs[0]\nzs = ys }}\nreport t"
        ),
    );
    costs(
        &format!("{LISTS}var ys = xs\nvar zs = ys\nvar t = 0\nrepeat 1000 {{ ys.push(1)\nt += len(zs) }}\nreport t"),
        &format!(
            "{LISTS}var ys = xs\nvar zs = ys\nvar t = 0\nrepeat 1000 {{ ys.push(1)\nt += len(zs)\nzs = ys }}\nreport t"
        ),
    );
}
