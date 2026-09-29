use super::*;
use crate::ErrorKind;
use std::io::Cursor;

/// Files in memory; `opened` records every open.
#[derive(Default)]
struct Memory {
    files: Vec<(String, Vec<u8>)>,
    opened: Vec<String>,
}

impl Memory {
    fn with(files: &[(&str, &str)]) -> Memory {
        Memory {
            files: files
                .iter()
                .map(|(p, t)| (p.to_string(), t.as_bytes().to_vec()))
                .collect(),
            opened: Vec::new(),
        }
    }
}

impl Resolver for Memory {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        if self.files.iter().any(|(p, _)| p == path) {
            Ok(format!("/data/{path}"))
        } else {
            Err("there's no such file".to_string())
        }
    }

    fn open(&mut self, identity: &str) -> Result<Box<dyn Read + Send>, String> {
        self.opened.push(identity.to_string());
        let path = identity.strip_prefix("/data/").unwrap();
        let bytes = self.files.iter().find(|(p, _)| p == path).unwrap().1.clone();
        Ok(Box::new(Cursor::new(bytes)))
    }
}

fn program(src: &str) -> Program {
    let (program, diags) = probl_sema::compile(src);
    program.unwrap_or_else(|| panic!("compile errors: {diags:?}"))
}

fn load_limited(src: &str, files: &[(&str, &str)], limits: &InputLimits) -> Result<Inputs, RuntimeError> {
    load(
        &program(src),
        &mut Memory::with(files),
        &mut Snapshots::default(),
        limits,
        None,
    )
}

/// The value of the program's first input.
fn value(src: &str, files: &[(&str, &str)]) -> Value {
    match load_limited(src, files, &InputLimits::default()) {
        Ok(inputs) => inputs.values()[0].clone(),
        Err(e) => panic!("{}", e.message),
    }
}

fn error(src: &str, files: &[(&str, &str)]) -> RuntimeError {
    load_limited(src, files, &InputLimits::default()).expect_err("expected an error")
}

#[track_caller]
fn fails(src: &str, files: &[(&str, &str)], expected: &[&str]) {
    let e = error(src, files);
    let text = format!("{} {:?} {:?}", e.message, e.notes, e.help);
    for part in expected {
        assert!(text.contains(part), "{part:?} isn't in: {text}");
    }
}

const DAY: &str = "type Day = { day: date, visitors: int, signups: int }\nlet pilot: list[Day] = read(\"pilot.csv\")\n";

#[test]
fn csv_rows_become_records() {
    let v = value(
        DAY,
        &[(
            "pilot.csv",
            "day,visitors,signups\n2026-09-01,250,9\n2026-09-02,250,6\n",
        )],
    );
    assert_eq!(
        v.to_string(),
        "[Day { day: 2026-09-01, signups: 9, visitors: 250 }, Day { day: 2026-09-02, signups: 6, visitors: 250 }]"
    );
}

#[test]
fn csv_names_match_ignoring_case_and_punctuation() {
    let src = "let rows: list[{ daily_sign_ups: int }] = read(\"a.csv\")";
    let v = value(src, &[("a.csv", "Date,Daily sign-ups,Notes\n2026-09-01,9,fine\n")]);
    assert_eq!(v.to_string(), "[{ daily_sign_ups: 9 }]");
    fails(
        src,
        &[("a.csv", "x,y\n1,2\n")],
        &["no column matches the field `daily_sign_ups`", "`x`, `y`"],
    );
    fails(
        src,
        &[("a.csv", "Daily sign-ups,daily_sign_ups\n1,2\n")],
        &["both match the field `daily_sign_ups`"],
    );
}

#[test]
fn csv_text_is_kept_as_written() {
    let src = "let rows: list[{ name: str, n: int }] = read(\"a.csv\")";
    let v = value(
        src,
        &[(
            "a.csv",
            "name,n\n ACME , 12 \n\" ACME \",3\n\"a, \"\"b\"\"\nc\",4\n,5\n",
        )],
    );
    let Value::List(rows) = v else { panic!() };
    let names: Vec<String> = rows
        .iter()
        .map(|r| match r {
            Value::Record(r) => r.get("name").unwrap().to_string(),
            _ => panic!(),
        })
        .collect();
    assert_eq!(names, [" ACME ", " ACME ", "a, \"b\"\nc", ""]);
}

#[test]
fn csv_errors_say_where() {
    let file = |t: &str| [("pilot.csv", t.to_string())];
    let check = |text: &str, expected: &[&str]| {
        let f = file(text);
        let files: Vec<(&str, &str)> = f.iter().map(|(p, t)| (*p, t.as_str())).collect();
        fails(DAY, &files, expected);
    };
    check(
        "day,visitors,signups\n2026-09-01,250,n/a\n",
        &["pilot.csv, line 2, column `signups`: `n/a` isn't an int"],
    );
    check(
        "day,visitors,signups\n2026-09-01,250\n",
        &["line 2", "this row has 2 cells, but the header has 3"],
    );
    check(
        "day,visitors,signups\n2026-09-01,250,\n",
        &["line 2", "the value is missing", "only a `str` field"],
    );
    check(
        "day,visitors,signups\n\"2026-09-01,250,9\n",
        &["line 2", "a quote isn't closed"],
    );
    check(
        "day,visitors,signups\n01/09/2026,250,9\n",
        &["isn't a date", "2027-01-31"],
    );
    check(
        "day,visitors,signups\n2026-09-01,2.5,9\n",
        &["`2.5` isn't a whole number", "`float`"],
    );
}

#[test]
fn csv_must_be_text() {
    let mut files = Memory::default();
    files.files.push((
        "pilot.csv".into(),
        b"day,visitors,signups\n2026-09-01,250,9\xff\n".to_vec(),
    ));
    let e = load(
        &program(DAY),
        &mut files,
        &mut Snapshots::default(),
        &InputLimits::default(),
        None,
    )
    .unwrap_err();
    assert!(
        e.message.contains("line 2") && e.message.contains("UTF-8"),
        "{}",
        e.message
    );
}

#[test]
fn plain_values_follow_their_type() {
    let src = "enum Size { Small, Large }\ntype R = { p: prob, f: float, b: bool, s: Size }\nlet rows: list[R] = read(\"a.csv\")";
    let v = value(
        src,
        &[("a.csv", "p,f,b,s\n30%,12,TRUE,Small\n0.25,-1.5e3,false,Large\n")],
    );
    assert_eq!(
        v.to_string(),
        "[R { b: true, f: 12.0, p: 30%, s: Small }, R { b: false, f: -1500.0, p: 25%, s: Large }]"
    );
    fails(
        src,
        &[("a.csv", "p,f,b,s\n30,1,true,Small\n")],
        &["isn't between 0 and 1", "did you mean `30%`?"],
    );
    fails(src, &[("a.csv", "p,f,b,s\n0.5,1,yes,Small\n")], &["`yes` isn't a bool"]);
    fails(
        src,
        &[("a.csv", "p,f,b,s\n0.5,1,true,Medium\n")],
        &["`Medium` isn't a `Size`", "Small, Large"],
    );
    fails(src, &[("a.csv", "p,f,b,s\n0.5,inf,true,Small\n")], &["isn't a number"]);
}

#[test]
fn large_integers_are_exact() {
    let big = 9_007_199_254_740_993_i64;
    let csv = value(
        "let rows: list[{ n: int }] = read(\"a.csv\")",
        &[("a.csv", "n\n9007199254740993\n")],
    );
    assert_eq!(csv.to_string(), format!("[{{ n: {big} }}]"));
    let lines = value(
        "let ns: list[int] = read(\"a.txt\")",
        &[("a.txt", "9007199254740993\n")],
    );
    assert_eq!(lines.to_string(), format!("[{big}]"));
    let json = value(
        "let ns: list[int] = read(\"a.json\")",
        &[("a.json", "[9007199254740993]")],
    );
    assert_eq!(json.to_string(), format!("[{big}]"));
    for n in [
        "18446744073709551616",
        "9223372036854775808",
        "123456789012345678901234567890123456789",
    ] {
        let data = format!("[{n}]");
        let v = value("let ns: list[int] = read(\"a.json\")", &[("a.json", &data)]);
        assert_eq!(v.to_string(), data);
    }
    fails(
        "let ns: list[int] = read(\"a.json\")",
        &[("a.json", "[12.0]")],
        &["`12.0` isn't an int"],
    );
}

#[test]
fn lines_are_values() {
    let v = value("let ns: list[int] = read(\"a.txt\")", &[("a.txt", "3\r\n\n 4 \n5")]);
    assert_eq!(v.to_string(), "[3, 4, 5]");
    let v = value("let ws: list[str] = read(\"a.txt\")", &[("a.txt", " a \nb\n")]);
    assert_eq!(format!("{v:?}"), "[\" a \", \"b\"]");
    fails(
        "let ns: list[int] = read(\"a.txt\")",
        &[("a.txt", "3\n\nx\n")],
        &["a.txt, line 3: `x` isn't an int"],
    );
}

const ASSUMPTIONS: &str = "enum Size { Small, Large }
type Segment = { name: Size, share: prob }
type Assumptions = { price: float, churn: prob, segments: list[Segment] }
let assumptions: Assumptions = read(\"assumptions.json\")
";

#[test]
fn json_reads_nested_named_types() {
    let v = value(
        ASSUMPTIONS,
        &[(
            "assumptions.json",
            r#"{ "price": 12.5, "churn": "4%", "segments": [ { "name": "Small", "share": 0.7 }, { "name": "Large", "share": 0.3 } ], "notes": [1, {"x": null}] }"#,
        )],
    );
    assert_eq!(
        v.to_string(),
        "Assumptions { churn: 4%, price: 12.5, segments: [Segment { name: Small, share: 70% }, Segment { name: Large, share: 30% }] }"
    );
}

#[test]
fn json_errors_say_where() {
    let check = |json: &str, expected: &[&str]| fails(ASSUMPTIONS, &[("assumptions.json", json)], expected);
    check(
        r#"{ "price": 12.5, "churn": "4%", "segments": [ { "name": "Small", "share": 0.7 }, { "name": "Large", "share": 3 } ] }"#,
        &[
            "assumptions.json, line 1, column",
            "at segments[1].share",
            "`3` isn't between 0 and 1",
        ],
    );
    check(
        r#"{ "price": 12.5, "price": 13, "churn": "4%", "segments": [] }"#,
        &["the key `price` appears twice"],
    );
    check(
        r#"{ "price": 12.5, "Price": 13, "churn": "4%", "segments": [] }"#,
        &["the keys `price` and `Price` both match the field `price`"],
    );
    check(
        r#"{ "price": 12.5, "segments": [] }"#,
        &["no key matches the field `churn`", "`price`, `segments`"],
    );
    check(
        r#"{ "price": null, "churn": "4%", "segments": [] }"#,
        &["found `null`", "at price"],
    );
    check(
        r#"{ "price": "12", "churn": "4%", "segments": [] }"#,
        &["found the string `12`", "without quotes"],
    );
    check(
        r#"{ "price": 12.5, "churn": "4%", "segments": {} }"#,
        &["expected a `list[Segment]`, found an object"],
    );
    check(
        r#"{ "price": 12.5, "churn": "4%", "segments": [] } x"#,
        &["isn't valid JSON", "trailing"],
    );
    check(r#"{ "price": 12.5, "#, &["isn't valid JSON"]);
    check(
        r#"{ "price": 1e400, "churn": "4%", "segments": [] }"#,
        &["too large for a float"],
    );
}

#[test]
fn arbitrary_integers_round_trip_and_obey_limits() {
    let n = "123456789012345678901234567890123456789012345678901234567890";
    let sources = [
        ("let x: list[int] = read(\"n.json\")", "n.json", format!("[{n}, -{n}]")),
        ("let x: list[int] = read(\"n.txt\")", "n.txt", format!("{n}\n-{n}\n")),
        (
            "let x: list[{ n: int }] = read(\"n.csv\")",
            "n.csv",
            format!("n\n{n}\n-{n}\n"),
        ),
    ];
    for (src, name, text) in &sources {
        let v = value(src, &[(name, text)]).to_string();
        assert!(v.contains(n) && v.contains(&format!("-{n}")), "{v}");
        for limits in [
            InputLimits {
                max_integer_bits: 64,
                ..InputLimits::default()
            },
            InputLimits {
                max_integer_bytes: 10,
                ..InputLimits::default()
            },
        ] {
            assert_eq!(
                load_limited(src, &[(name, text)], &limits).unwrap_err().kind,
                ErrorKind::Limit
            );
        }
    }
    let src = "let x: map[int, int] = read(\"n.json\")";
    let json = format!("{{\"{n}\": {n}}}");
    assert_eq!(value(src, &[("n.json", &json)]).to_string(), format!("[{n}: {n}]"));
    let duplicate = format!("{{\"{n}\": 1, \"0{n}\": 2}}");
    assert!(
        error(src, &[("n.json", &duplicate)])
            .message
            .contains("are the same int")
    );
    for ty in ["int", "float", "prob"] {
        fails(
            &format!("let x: {ty} = read(\"n.json\")"),
            &[(
                "n.json",
                r#"{"$serde_json::private::Number":"123456789012345678901234567890"}"#,
            )],
            &["object"],
        );
    }
    let oversized = format!("[{}]", "9".repeat(20_000));
    assert_eq!(error(sources[0].0, &[("n.json", &oversized)]).kind, ErrorKind::Limit);
    for (format, text) in [
        (DataFormat::Json, format!("[{n}]")),
        (DataFormat::Lines, n.to_string()),
        (DataFormat::Csv, format!("n\n{n}\n")),
    ] {
        let schema = suggest(text.as_bytes(), format, "n", &InputLimits::default()).unwrap();
        assert!(schema.contains("int"), "{schema}");
    }
    // A host may tighten limits after loading data, without letting a direct
    // report of the input bypass the runtime limit.
    let program = program(sources[0].0);
    let inputs = load(
        &program,
        &mut Memory::with(&[("n.json", &sources[0].2)]),
        &mut Snapshots::default(),
        &InputLimits::default(),
        None,
    )
    .unwrap();
    let options = crate::Options {
        inputs: Some(Arc::new(inputs)),
        limits: crate::Limits {
            max_integer_bits: 64,
            ..crate::Limits::default()
        },
        ..crate::Options::default()
    };
    assert_eq!(
        crate::run(&program, &options, &mut |_| {}).unwrap_err().kind,
        ErrorKind::Limit
    );
}

#[test]
fn json_maps_keep_their_keys() {
    let v = value(
        "let m: map[date, float] = read(\"a.json\")",
        &[("a.json", r#"{"2027-01-02": 1.5, "2027-01-01": 2}"#)],
    );
    assert_eq!(v.to_string(), "[2027-01-01: 2.0, 2027-01-02: 1.5]");
    // Map keys aren't field names: they're kept as they are.
    let v = value(
        "let m: map[str, int] = read(\"a.json\")",
        &[("a.json", r#"{"A b": 1, "ab": 2}"#)],
    );
    assert_eq!(format!("{v:?}"), "[\"A b\": 1, \"ab\": 2]");
    fails(
        "let m: map[int, str] = read(\"a.json\")",
        &[("a.json", r#"{"1": "first", "01": "second"}"#)],
        &["the keys `1` and `01` are the same int"],
    );
    fails(
        "let m: map[int, str] = read(\"a.json\")",
        &[("a.json", r#"{"1": "first", "1": "second"}"#)],
        &["the key `1` appears twice"],
    );
}

#[test]
fn json_bags_are_items_or_counts() {
    let v = value(
        "let b: bag[str] = read(\"a.json\")",
        &[("a.json", r#"["ace", "king", "ace"]"#)],
    );
    assert_eq!(format!("{v:?}"), "bag([\"ace\": 2, \"king\": 1])");
    let v = value(
        "let b: bag[str] = read(\"a.json\")",
        &[("a.json", r#"{"ace": 4, "king": 0}"#)],
    );
    assert_eq!(format!("{v:?}"), "bag([\"ace\": 4])");
    fails(
        "let b: bag[str] = read(\"a.json\")",
        &[("a.json", r#"{"ace": -1}"#)],
        &["can't be negative"],
    );
    fails(
        "let b: bag[{ n: int }] = read(\"a.json\")",
        &[("a.json", r#"{"ace": 1}"#)],
        &["is written as an array of its items"],
    );
}

#[test]
fn limits_count_all_the_data() {
    let src = "let a: list[int] = read(\"a.txt\")\nlet b: list[int] = read(\"b.txt\")";
    let files = [("a.txt", "1\n2\n3\n"), ("b.txt", "4\n5\n6\n")];
    let limits = |f: &dyn Fn(&mut InputLimits)| {
        let mut l = InputLimits::default();
        f(&mut l);
        l
    };
    assert!(load_limited(src, &files, &limits(&|l| l.max_bytes = 12)).is_ok());
    let e = load_limited(src, &files, &limits(&|l| l.max_bytes = 11)).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Limit);
    assert!(e.message.starts_with("b.txt: the data is more than"), "{}", e.message);
    let e = load_limited(src, &files, &limits(&|l| l.max_values = 5)).unwrap_err();
    assert!(e.message.contains("more than 5 values"), "{}", e.message);
    let e = load_limited(src, &files, &limits(&|l| l.max_collection = 2)).unwrap_err();
    assert!(e.message.contains("more than 2 elements"), "{}", e.message);
    let deep = "[".repeat(10) + &"]".repeat(10);
    let e = load_limited(
        "let x: list[list[list[list[list[list[list[list[list[list[int]]]]]]]]]] = read(\"a.json\")",
        &[("a.json", deep.as_str())],
        &limits(&|l| l.max_depth = 4),
    )
    .unwrap_err();
    assert!(e.message.contains("nests more than 4 levels"), "{}", e.message);
}

#[test]
fn loading_can_be_cancelled() {
    let cancel = AtomicBool::new(true);
    let e = load(
        &program("let a: list[int] = read(\"a.txt\")"),
        &mut Memory::with(&[("a.txt", "1\n")]),
        &mut Snapshots::default(),
        &InputLimits::default(),
        Some(&cancel),
    )
    .unwrap_err();
    assert!(e.message.contains("cancelled"), "{}", e.message);
}

/// A stream that fails after a few bytes.
struct Broken(usize);

impl Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::Error::other("the connection was reset"));
        }
        self.0 -= 1;
        buf[0] = b'1';
        Ok(1)
    }
}

struct Refusing;

impl Resolver for Refusing {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        match path {
            "broken.txt" => Ok(path.to_string()),
            _ => Err("this host only reads files you upload".to_string()),
        }
    }

    fn open(&mut self, _: &str) -> Result<Box<dyn Read + Send>, String> {
        Ok(Box::new(Broken(3)))
    }
}

#[test]
fn hosts_decide_what_can_be_read() {
    let load_one = |src: &str| {
        load(
            &program(src),
            &mut Refusing,
            &mut Snapshots::default(),
            &InputLimits::default(),
            None,
        )
        .unwrap_err()
    };
    let e = load_one("let a: list[int] = read(\"/etc/passwd.txt\")");
    assert_eq!(
        e.message,
        "/etc/passwd.txt: can't read it: this host only reads files you upload"
    );
    let e = load_one("let a: list[int] = read(\"broken.txt\")");
    assert!(
        e.message.contains("reading it failed: the connection was reset"),
        "{}",
        e.message
    );
}

#[test]
fn a_source_is_read_once() {
    let src =
        "let a: list[int] = read(\"n.txt\")\nlet b: list[str] = read(\"n.txt\")\nlet c: list[float] = read(\"n.txt\")";
    let mut files = Memory::with(&[("n.txt", "7\n8\n")]);
    let mut snapshots = Snapshots::default();
    let inputs = load(&program(src), &mut files, &mut snapshots, &InputLimits::default(), None).unwrap();
    assert_eq!(files.opened, ["/data/n.txt"]);
    let values: Vec<String> = inputs.values().iter().map(|v| format!("{v:?}")).collect();
    assert_eq!(values, ["[7, 8]", "[\"7\", \"8\"]", "[7.0, 8.0]"]);
    assert_eq!(inputs.sources().len(), 1);
    assert_eq!(inputs.sources()[0].bytes, 4);
    assert_eq!(
        inputs.sources()[0].sha256,
        "adddb5146a8578f617ab1f5e103c545490e99463d45932ea9375acc4cf905ec1"
    );
    // A snapshot kept between loads keeps the data, even if the file changes.
    files.files[0].1 = b"1\n".to_vec();
    let again = load(&program(src), &mut files, &mut snapshots, &InputLimits::default(), None).unwrap();
    assert_eq!(files.opened.len(), 1);
    assert_eq!(format!("{:?}", again.values()[0]), "[7, 8]");
    snapshots.clear();
    let reloaded = load(&program(src), &mut files, &mut snapshots, &InputLimits::default(), None).unwrap();
    assert_eq!(files.opened.len(), 2);
    assert_eq!(format!("{:?}", reloaded.values()[0]), "[1]");
}

#[test]
fn inputs_belong_to_their_program() {
    let src = "let a: list[int] = read(\"n.txt\")";
    let inputs = load_limited(src, &[("n.txt", "7\n")], &InputLimits::default()).unwrap();
    assert!(inputs.fit(&program(src)));
    assert!(!inputs.fit(&program("let a: list[float] = read(\"n.txt\")")));
    assert!(!inputs.fit(&program("let b: list[int] = read(\"n.txt\")")));
}

#[test]
fn schemas_are_suggested() {
    let limits = InputLimits::default();
    let csv = suggest(
        b"Day,Visitors,Daily sign-ups,rate,ok,type,2026 total,note\n2026-09-01,250,9,3%,true,a,1,\n2026-09-02,250,6.5,4%,false,b,2,x\n",
        DataFormat::Csv,
        "data/pilot-2026.csv",
        &limits,
    )
    .unwrap();
    assert_eq!(
        csv,
        "# the column \"2026 total\" can't be a field name: rename it in the file to read it\n\
         type Pilot2026 = { day: date, visitors: int, daily_sign_ups: float, rate: prob, ok: bool, type_: str, note: str }\n\
         # let pilot_2026: list[Pilot2026] = read(\"data/pilot-2026.csv\")\n"
    );
    let json = suggest(
        br#"{"price": 12.5, "churn": "4%", "startDate": "2027-01-01", "segments": [{"name": "Small", "share": 0.7}, {"name": "Large", "share": 1}], "daily": {"2027-01-01": 3, "2027-01-02": 4}}"#,
        DataFormat::Json,
        "assumptions.json",
        &limits,
    )
    .unwrap();
    assert_eq!(
        json,
        "type Assumptions = { price: float, churn: prob, start_date: date, segments: list[{ name: str, share: float }], daily: map[date, int] }\n\
         # let assumptions: Assumptions = read(\"assumptions.json\")\n"
    );
    let lines = suggest(b"1\n2.5\n\n", DataFormat::Lines, "-", &limits).unwrap();
    assert_eq!(lines, "# let data: list[float] = read(\"-\")\n");
    let small = InputLimits {
        max_values: 3,
        ..InputLimits::default()
    };
    assert!(suggest(b"1\n2\n3\n4\n", DataFormat::Lines, "a.txt", &small).is_err());
}
