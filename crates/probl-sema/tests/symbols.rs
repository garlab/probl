//! Where names are declared and used, as editors see it: going to a
//! definition, and the names that can be used at a point.

use probl_sema::symbols::{DefKind, Symbols};
use probl_syntax::Span;

const SRC: &str = "\
enum Market { Boom, Steady, Slump }
type Day = { visitors: int, signups: int }

let price = 49
var market = Boom

fn earn(customers: int) -> int {
  let total = customers * price
  return total
}

let x = 1
for day in [Day { visitors: 10, signups: 2 }] {
  let rate = day.signups / day.visitors
  let x = x + rate
  market = Market.Steady
}
let doubled = map([1, 2], n -> n * x)
let label = match market {
  Boom => \"up\"
  other => \"steady or down\"
}
report earn(x)
";

fn symbols() -> Symbols {
    let (program, diags, symbols) = probl_sema::compile_with_symbols(SRC);
    assert!(program.is_some(), "{diags:?}");
    symbols.unwrap()
}

/// The offset of the `n`th occurrence of `needle` (from 0), plus `at`.
fn offset(needle: &str, n: usize, at: usize) -> u32 {
    let mut from = 0;
    for _ in 0..n {
        from += SRC[from..].find(needle).unwrap() + 1;
    }
    (from + SRC[from..].find(needle).unwrap() + at) as u32
}

/// The text of the declaration that the name at `offset` refers to, with
/// the line it's on.
fn definition(symbols: &Symbols, offset: u32) -> (String, usize) {
    let d = &symbols.definitions[symbols.at(offset).expect("a name")];
    let line = SRC[..d.span.lo as usize].lines().count().max(1);
    (SRC[d.span.lo as usize..d.span.hi as usize].to_string(), line)
}

#[test]
fn uses_lead_to_their_declarations() {
    let s = symbols();
    // A global read inside a function.
    assert_eq!(definition(&s, offset("price", 1, 0)), ("price".into(), 4));
    // A parameter, and a local.
    assert_eq!(definition(&s, offset("customers", 1, 0)), ("customers".into(), 7));
    assert_eq!(definition(&s, offset("total", 1, 0)), ("total".into(), 8));
    // Shadowing in the loop: `x + rate` reads the top-level `x`, as does
    // everything after the loop.
    assert_eq!(definition(&s, offset("x + rate", 0, 0)), ("x".into(), 12));
    assert_eq!(definition(&s, offset("n * x", 0, 4)), ("x".into(), 12));
    assert_eq!(definition(&s, offset("earn(x)", 0, 5)), ("x".into(), 12));
    // A loop variable and a lambda's parameter.
    assert_eq!(definition(&s, offset("day.signups", 0, 0)), ("day".into(), 13));
    assert_eq!(definition(&s, offset("n * x", 0, 0)), ("n".into(), 18));
    // A function, a record type, an enum and its variants.
    assert_eq!(definition(&s, offset("earn(x)", 0, 0)), ("earn".into(), 7));
    assert_eq!(definition(&s, offset("Day {", 0, 0)), ("Day".into(), 2));
    assert_eq!(definition(&s, offset("Market.Steady", 0, 0)), ("Market".into(), 1));
    assert_eq!(definition(&s, offset("Market.Steady", 0, 7)), ("Steady".into(), 1));
    assert_eq!(definition(&s, offset("= Boom", 0, 2)), ("Boom".into(), 1));
    assert_eq!(definition(&s, offset("Boom =>", 0, 0)), ("Boom".into(), 1));
    // Fields: in a record with its type, and read from a value.
    assert_eq!(definition(&s, offset("visitors: 10", 0, 0)), ("visitors".into(), 2));
    assert_eq!(definition(&s, offset("day.signups", 0, 4)), ("signups".into(), 2));
    // An assignment, and a declaration itself.
    assert_eq!(definition(&s, offset("market = Market", 0, 0)), ("market".into(), 5));
    assert_eq!(definition(&s, offset("label", 0, 1)), ("label".into(), 19));
}

#[test]
fn names_are_visible_where_they_can_be_used() {
    let s = symbols();
    assert_eq!(
        s.functions,
        [Span::new(
            offset("fn earn", 0, 0) as usize,
            offset("}\n\nlet x", 0, 1) as usize
        )]
    );
    let names = |at: u32| -> Vec<String> {
        s.visible(at)
            .into_iter()
            .map(|i| s.definitions[i].name.clone())
            .collect()
    };
    let in_loop = names(offset("market = Market", 0, 0));
    for name in ["rate", "day", "x", "price", "market", "earn", "Boom", "Market", "Day"] {
        assert!(in_loop.contains(&name.to_string()), "{name} in {in_loop:?}");
    }
    assert!(!in_loop.contains(&"total".to_string()), "a function's local");
    assert!(!in_loop.contains(&"doubled".to_string()), "declared later");
    // After the loop, its variables are gone.
    let after = names(offset("let doubled", 0, 0));
    assert!(
        !after.contains(&"rate".to_string()) && !after.contains(&"day".to_string()),
        "{after:?}"
    );
    // In a function: its parameter, and every top-level name, even later ones.
    let in_function = names(offset("return total", 0, 0));
    for name in ["customers", "total", "price", "label", "doubled"] {
        assert!(in_function.contains(&name.to_string()), "{name} in {in_function:?}");
    }
    // Nearest first: the loop's `x` before the top-level one.
    let xs: Vec<u32> = s
        .visible(offset("market = Market", 0, 0))
        .into_iter()
        .filter(|&i| s.definitions[i].name == "x")
        .map(|i| s.definitions[i].span.lo)
        .collect();
    assert_eq!(xs, [offset("let x = x", 0, 4), offset("let x = 1", 0, 4)]);
}

#[test]
fn definitions_know_their_kind() {
    let s = symbols();
    let find = |name: &str| s.definitions.iter().find(|d| d.name == name).unwrap();
    assert_eq!(find("earn").kind, DefKind::Function);
    assert_eq!(find("customers").kind, DefKind::Parameter);
    assert!(find("market").mutable && find("market").global);
    assert!(!find("total").global);
    let signups = find("signups");
    assert_eq!(signups.kind, DefKind::Field);
    assert_eq!(signups.owner.as_deref(), Some("Day"));
    assert_eq!(signups.ty.as_deref(), Some("int"));
    assert_eq!(find("Slump").owner.as_deref(), Some("Market"));
}

#[test]
fn a_program_that_doesnt_parse_has_none() {
    let (_, _, symbols) = probl_sema::compile_with_symbols("let x = (");
    assert!(symbols.is_none());
    // Other errors still have them.
    let (program, _, symbols) = probl_sema::compile_with_symbols("let x = 1\nreport y + x");
    assert!(program.is_none());
    assert_eq!(symbols.unwrap().references.len(), 1);
}

#[test]
fn fields_lead_to_their_record_when_its_certain() {
    let src = "\
type Fighter = { hp: int, ac: int, won: bool }
type Monster = { hp: int }
let hero = Fighter { hp: 12, ac: 14, won: false }
let tougher = hero with { ac: 16 }
let outcome = { won: true }
report tougher.ac
report hero.hp
report outcome.won
";
    let (program, diags, symbols) = probl_sema::compile_with_symbols(src);
    assert!(program.is_some(), "{diags:?}");
    let s = symbols.unwrap();
    let at = |needle: &str, skip: usize| (src.find(needle).unwrap() + skip) as u32;
    let field = |offset: u32| {
        s.at(offset).map(|i| {
            let d = &s.definitions[i];
            format!("{}.{}", d.owner.as_deref().unwrap_or("?"), d.name)
        })
    };
    // In a record with its type, each field is that type's.
    assert_eq!(field(at("hp: 12", 0)).as_deref(), Some("Fighter.hp"));
    assert_eq!(field(at("won: false", 0)).as_deref(), Some("Fighter.won"));
    // Elsewhere, a field only one record has.
    assert_eq!(field(at("ac: 16", 0)).as_deref(), Some("Fighter.ac"));
    assert_eq!(field(at("tougher.ac", 8)).as_deref(), Some("Fighter.ac"));
    // Not one two records have, or one a record without a type has.
    assert_eq!(field(at("hero.hp", 5)), None);
    assert_eq!(field(at("outcome.won", 8)), None);
}
