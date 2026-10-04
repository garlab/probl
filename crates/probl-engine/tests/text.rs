//! Unicode scalar sequences, immutable transformations and bounded text work.
mod common;

use common::*;
use probl_engine::builtins::call_plain;
use probl_engine::dist::Budget;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, Limits, Options};
use probl_sema::Builtin as B;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

#[track_caller]
fn facts(expressions: &[&str]) {
    for expression in expressions {
        assert_eq!(chance(&format!("report {expression}")), 1.0, "{expression}");
    }
}

#[test]
fn strings_are_exact_sequences_of_unicode_scalars() {
    facts(&[
        r#"len("é🙂🇫🇷") == 4"#,
        r#"chars("é") == ["e", "́"]"#,
        r#"len("👩‍💻") == 3"#,
        r#""a🙂bc"[1] == "🙂""#,
        r#""é"[1] == "́""#,
        r#"chars("") == []"#,
        r#"chars("a\0b") == ["a", "\0", "b"]"#,
        r#"chars("a🙂é").join("") == "a🙂é""#,
        r#"reverse("é") == "́e""#,
        r#""é" != "é""#,
        r#"len(["é": 1, "é": 2]) == 2"#,
        r#"sort(["🙂", "é", "a"]) == ["a", "é", "🙂"]"#,
        r#"upper("Straße") == "STRASSE""#,
        r#"lower("İ") == "i̇""#,
        r#"lower("ΟΣ ΟΣΑ") == "ος οσα""#,
    ]);
    assert_eq!(distribution(r#"report one_of(["é", "é"])"#).len(), 2);
    facts(&[r#"{ var result = ""; for c in "a🙂é" { result += c }; result } == "a🙂é""#]);
}

#[test]
fn trim_uses_unicode_whitespace_or_an_explicit_scalar_set() {
    facts(&[
        r#""__foo__".trim("_") == "foo""#,
        r#""abbacacb".trim("ab") == "cac""#,
        r#""abbacacb".trim_start("ab") == "cacb""#,
        r#""abbacacb".trim_end("ab") == "abbacac""#,
        r#"trim(" \t\r\n  hello \n") == "hello""#,
        r#"trim_start(" \thello\n") == "hello\n""#,
        r#"trim_end(" \thello\n") == " \thello""#,
        "trim(\"\u{200b}hello\u{200b}\") == \"\u{200b}hello\u{200b}\"",
        r#"trim(" x ", "") == " x ""#,
        r#"trim_start(" x ", "") == " x ""#,
        r#"trim_end(" x ", "") == " x ""#,
        r#"trim(" x ", "x") == " x ""#,
        r#"trim("ababa", "aabb") == """#,
        r#"trim("🙂étexté🙂", "é🙂") == "text""#,
        r#"trim("é", "é") == "é""#,
        r#"trim("é", "́") == "e""#,
        r#"trim("\0hello\0", "\0") == "hello""#,
        r#"trim("") == "" and trim("", "ab") == """#,
        r#"{ var s = "__foo__"; let t = s.trim("_"); s == "__foo__" and t == "foo" }"#,
    ]);
}

#[test]
fn literal_search_and_split_keep_their_exact_meaning() {
    facts(&[
        r#"starts_with("🙂abc", "🙂a")"#,
        r#"ends_with("abc🙂", "c🙂")"#,
        r#"starts_with("", "") and ends_with("", "")"#,
        r#"starts_with("abc", "") and ends_with("abc", "")"#,
        r#"not starts_with("a", "ab") and not ends_with("a", "ba")"#,
        r#"not starts_with("é", "é") and not ends_with("é", "é")"#,
        r#""́" in "é""#,
        r#""" in """#,
        r#"split("a,,b,", ",") == ["a", "", "b", ""]"#,
        r#"split("", ",") == [""]"#,
        r#"split("", "") == []"#,
        r#"split("é🙂", "") == chars("é🙂")"#,
        r#"split("a.b", ".") == ["a", "b"]"#,
        r#"split("a🙂b🙂", "🙂") == ["a", "b", ""]"#,
    ]);
}

#[test]
fn slices_preserve_types_and_use_strict_exclusive_bounds() {
    facts(&[
        r#"slice("a🙂bc", 1, 3) == "🙂b""#,
        r#"slice("a🙂bc", 1) == "🙂bc""#,
        r#"slice("é", 1, 2) == "́""#,
        r#"slice("", 0) == """#,
        r#"slice("abc", 3, 3) == """#,
        r#"slice([10, 20, 30], 1) == [20, 30]"#,
        r#"slice([10, 20, 30], 0, 3) == [10, 20, 30]"#,
        r#"slice([], 0, 0) == []"#,
        r#"slice([1, 2], 1, 1) == []"#,
        r#"slice(10..20, 2, 5) == (12..14)"#,
        r#"slice(-10..10, 9, 12) == (-1..1)"#,
        r#"slice(10..20, 2) == (12..20)"#,
        r#"len(slice(10..20, 11)) == 0"#,
        r#"len(slice(0..<0, 0)) == 0"#,
        r#"slice(0..10^100, 10^100)[0] == 10^100"#,
        r#"len(slice(0..10^100, 1, 10^100)) == 10^100 - 1"#,
        r#"{ var xs = [1, 2, 3]; var ys = xs.slice(1); ys[0] = 9; xs == [1, 2, 3] and ys == [9, 3] }"#,
    ]);
    for xs in [r#""abc""#, "[1, 2, 3]", "10..12"] {
        for bounds in ["-1", "0, -1", "2, 1", "0, 4", "4", "10^100", "0, 10^100"] {
            assert!(error(&format!("report slice({xs}, {bounds})")).contains("0 <= start"));
        }
        for bounds in ["0.4", "0, 2.4", "true", "0, 50%", "complex(0)"] {
            assert!(error(&format!("report slice({xs}, {bounds})")).contains("needs an int"));
        }
    }
    // The exclusive endpoint needn't be representable as a range endpoint.
    let max = format!("0x{}", "f".repeat(16384));
    facts(&[&format!("slice({max}..{max}, 0, 1)[0] == {max}")]);
    facts(&[&format!("len(slice({max}..{max}, 1)) == 0")]);
}

#[test]
fn collection_algorithms_share_scalar_iteration_and_return_lists() {
    facts(&[
        r#"map("a🙂é", c -> len(c)) == [1, 1, 1]"#,
        r#"map("aß", c -> upper(c)) == ["A", "SS"]"#,
        r#"filter("abca", c -> c != "a") == ["b", "c"]"#,
        r#""abca".filter(c -> c != "a").join("") == "bc""#,
        r#"map("", c -> c) == [] and filter("", c -> true) == []"#,
        r#"reduce("a🙂é", (s, c) -> s + c, "") == "a🙂é""#,
        r#"count("a🙂a", c -> c == "a") == 2"#,
        r#"sort("baé") == ["a", "b", "é"]"#,
        r#"sort_desc("baé") == ["é", "b", "a"]"#,
        r#"enumerate("a🙂") == [[0, "a"], [1, "🙂"]]"#,
        r#"zip("a🙂b", 10..11) == [["a", 10], ["🙂", 11]]"#,
        r#"zip([1, 2], "é") == [[1, "é"]]"#,
        r#"join([1, true, "🙂"], ",") == "1,true,🙂""#,
    ]);
}

#[test]
fn text_functions_lift_over_distributions() {
    assert_eq!(
        distribution(r#"report trim(one_of(["_a_", "_b_"]), "_")"#),
        vec![(Value::str("a"), 0.5), (Value::str("b"), 0.5)]
    );
    assert_eq!(
        distribution(r#"report slice("a🙂b", one_of([0, 1]), 2)"#),
        vec![(Value::str("a🙂"), 0.5), (Value::str("🙂"), 0.5)]
    );
    for mode in ["enumerate", "sample(runs: 100, seed: 3)"] {
        for expression in [
            r#"trim("_a_", one_of(["_", "__"])) == "a""#,
            r#"trim_start(one_of([" a", "  a"])) == "a""#,
            r#"trim_end(one_of(["a ", "a  "])) == "a""#,
            r#"starts_with(one_of(["abc", "abd"]), "ab")"#,
            r#"ends_with("🙂x", one_of(["x", "🙂x"]))"#,
            r#"chars(one_of(["a🙂", "b🙂"]))[1] == "🙂""#,
            r#"slice(one_of(["a🙂", "b🙂"]), 1) == "🙂""#,
            r#"map(one_of(["ab", "cd"]), c -> len(c)) == [1, 1]"#,
            r#"filter(one_of(["a🙂", "b🙂"]), c -> c == "🙂") == ["🙂"]"#,
        ] {
            assert_eq!(
                chance(&format!("@mode {mode}\nreport {expression}")),
                1.0,
                "{expression}"
            );
        }
    }
}

#[test]
fn text_arguments_and_arities_are_checked() {
    for name in ["trim", "trim_start", "trim_end", "chars", "starts_with", "ends_with"] {
        let binary = matches!(name, "starts_with" | "ends_with");
        for input in ["1", "1.0", "true", "[]", "complex(0)"] {
            let args = if binary {
                format!(r#"{input}, "a""#)
            } else {
                input.to_owned()
            };
            assert!(error(&format!("report {name}({args})")).contains("needs a string"));
            if name != "chars" {
                assert!(error(&format!(r#"report {name}("abc", {input})"#)).contains("needs a string"));
            }
        }
        assert!(compile_error(&format!("report {name}()")).contains("takes"));
        assert!(compile_error(&format!(r#"report {name}("a", "b", "c")"#)).contains("takes"));
    }
    for args in ["", "[]", "[], 0, 0, 0"] {
        assert!(compile_error(&format!("report slice({args})")).contains("takes"));
    }
    for value in ["1", "true", "[:]", "bag([1])"] {
        assert!(error(&format!("report slice({value}, 0)")).contains("list, range or string"));
    }
}

#[test]
fn string_size_limits_cover_all_construction_paths() {
    let options = Options {
        limits: Limits {
            max_string_bytes: 5,
            ..Limits::default()
        },
        ..Options::default()
    };
    for src in [
        r#"report "🙂é""#,
        r#"report "abc" + "def""#,
        r#"report upper("ΐΐ")"#,
        r#"report lower("İİ")"#,
        r#"report join(["abc", "def"], "")"#,
        r#"report str([1, 2])"#,
        r#"let xs = [1, 2]; report "{xs}""#,
        r#"let x = "abc"; report "{x}{x}""#,
        r#"print([1, 2])"#,
    ] {
        let err = exec_raw(src, &options).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Limit, "{src}: {err:?}");
        assert!(err.message.contains("string size"), "{src}: {err:?}");
    }
    assert!(exec_raw(r#"report "a🙂""#, &options).is_ok());
    // Limits use the actual case-mapped byte length, not a pessimistic multiple.
    assert!(exec_raw(r#"report upper("abcde")"#, &options).is_ok());
}

#[test]
fn string_scans_and_results_obey_work_and_memory_limits() {
    let s = Value::str(&"a".repeat(4096));
    for (f, args) in [
        (B::Len, vec![s.clone()]),
        (B::Trim, vec![s.clone()]),
        (B::Slice, vec![s.clone(), Value::Int(0.into())]),
        (B::Chars, vec![s.clone()]),
        (B::Split, vec![s.clone(), Value::str(",")]),
        (B::Upper, vec![s.clone()]),
        (B::StartsWith, vec![s.clone(), Value::str("b")]),
    ] {
        let mut budget = Budget {
            work_left: 1,
            ..Budget::unlimited()
        };
        assert_eq!(call_plain(f, &args, &mut budget).unwrap_err().kind, ErrorKind::Limit);
    }
    for f in [B::Trim, B::Upper, B::Lower, B::Chars, B::Reverse, B::Str] {
        let mut budget = Budget {
            string_bytes_left: Arc::new(AtomicU64::new(1)),
            ..Budget::unlimited()
        };
        let err = call_plain(f, std::slice::from_ref(&s), &mut budget).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Limit);
        assert!(err.message.contains("string memory allowance"));
    }
    let mut budget = Budget {
        work_left: 1,
        ..Budget::unlimited()
    };
    assert_eq!(
        probl_engine::ops::index(&s, &Value::Int(0.into()), &mut budget)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    let mut budget = Budget {
        work_left: 1,
        ..Budget::unlimited()
    };
    assert_eq!(
        probl_engine::ops::contains(&s, &Value::str("b"), &mut budget)
            .unwrap_err()
            .kind,
        ErrorKind::Limit
    );
    let mut budget = Budget {
        string_bytes_left: Arc::new(AtomicU64::new(3)),
        ..Budget::unlimited()
    };
    let mut worker = budget.clone();
    call_plain(B::Trim, &[Value::str("ab")], &mut budget).unwrap();
    assert_eq!(
        call_plain(B::Trim, &[Value::str("ab")], &mut worker).unwrap_err().kind,
        ErrorKind::Limit
    );
}

#[test]
fn materializing_text_checks_collection_limits_but_slicing_ranges_stays_compact() {
    let options = Options {
        limits: Limits {
            max_collection: 2,
            ..Limits::default()
        },
        ..Options::default()
    };
    for src in [
        r#"report chars("abc")"#,
        r#"report split("abc", "")"#,
        r#"report split("a,b,c", ",")"#,
        r#"report map("abc", c -> c)"#,
        r#"report filter("abc", c -> true)"#,
        r#"report enumerate("abc")"#,
        r#"report zip("abc", "xy")"#,
        r#"var n = 0; for c in "abc" { n += 1 }; report n"#,
        r#"report [1, 2] + [3]"#,
    ] {
        assert_eq!(exec_raw(src, &options).unwrap_err().kind, ErrorKind::Limit, "{src}");
    }
    assert!(exec_raw("report len(slice(0..10^100, 1)) == 10^100", &options).is_ok());
    assert!(exec_raw(r#"report slice("abc", 1) == "bc""#, &options).is_ok());
}
