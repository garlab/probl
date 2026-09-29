//! The data readers against random data (docs/data-input.md): what's
//! written reads back as the same values, and malformed data never makes
//! them panic, hang or report an internal error. `PROBL_DATA_CASES` sets
//! the number of cases (default 2,000).

use probl_engine::data::{InputLimits, Resolver, Snapshots, load, suggest};
use probl_engine::dates;
use probl_engine::ops::make_record;
use probl_engine::value::Value;
use probl_engine::{ErrorKind, RuntimeError};
use probl_sema::ir::DataFormat;
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::Arc;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

fn cases() -> u64 {
    std::env::var("PROBL_DATA_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000)
}

struct Bytes(Vec<u8>);

impl Resolver for Bytes {
    fn resolve(&mut self, path: &str) -> Result<String, String> {
        Ok(path.to_string())
    }

    fn open(&mut self, _: &str) -> Result<Box<dyn Read + Send>, String> {
        Ok(Box::new(Cursor::new(self.0.clone())))
    }
}

fn read(src: &str, bytes: Vec<u8>, limits: &InputLimits) -> Result<Value, RuntimeError> {
    let (program, diags) = probl_sema::compile(src);
    let program = program.unwrap_or_else(|| panic!("{diags:?}"));
    let inputs = load(&program, &mut Bytes(bytes), &mut Snapshots::default(), limits, None)?;
    Ok(inputs.values()[0].clone())
}

// ── Written data reads back ──────────────────────────────────────────────

fn text(rng: &mut Rng) -> String {
    let pieces = [
        "a", "B", " ", ",", "\"", "\n", "\r\n", "é", "日本", "x y", "'", "{", "}", "\\", "0", "%",
    ];
    (0..rng.below(6)).map(|_| *rng.pick(&pieces)).collect()
}

fn int(rng: &mut Rng) -> i64 {
    match rng.below(4) {
        0 => *rng.pick(&[i64::MIN, i64::MAX, 0, -1, 9_007_199_254_740_993]),
        1 => rng.next() as i64,
        _ => rng.below(2_000) as i64 - 1_000,
    }
}

fn float(rng: &mut Rng) -> f64 {
    match rng.below(3) {
        0 => *rng.pick(&[0.0, -0.5, 1e-300, 1.7976931348623157e308, 0.1 + 0.2]),
        1 => Some(f64::from_bits(rng.next()))
            .filter(|x| x.is_finite())
            .map_or(1.5, |x| x.clamp(-1e300, 1e300)),
        _ => (rng.below(1_000_000) as f64 - 500_000.0) / 7.0,
    }
}

/// A CSV cell: quoted when it must be, and sometimes when it needn't be.
fn cell(text: &str, rng: &mut Rng) -> String {
    if text.contains([',', '"', '\n', '\r']) || rng.below(4) == 0 {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

#[test]
fn written_data_reads_back() {
    let src_csv =
        "type Row = { name: str, n: int, x: float, ok: bool, day: date }\nlet rows: list[Row] = read(\"t.csv\")";
    let src_json = "type Row = { name: str, n: int, x: float, ok: bool, day: date, tags: list[str], counts: map[str, int] }\nlet rows: list[Row] = read(\"t.json\")";
    let names: Vec<Arc<str>> = ["counts", "day", "n", "name", "ok", "tags", "x"]
        .iter()
        .map(|&s| Arc::from(s))
        .collect();
    let ty: Option<Arc<str>> = Some(Arc::from("Row"));
    let mut rng = Rng(1);
    for case in 0..cases() / 4 {
        let rows = rng.below(5);
        let mut csv = String::from("name,n,x,ok,day\n");
        let mut json = Vec::new();
        let (mut csv_rows, mut json_rows) = (Vec::new(), Vec::new());
        for _ in 0..rows {
            let (name, n, x, ok) = (text(&mut rng), int(&mut rng), float(&mut rng), rng.below(2) == 0);
            let day = rng.below(40_000) as i32 - 5_000;
            let tags: Vec<String> = (0..rng.below(3)).map(|_| text(&mut rng)).collect();
            let counts: BTreeMap<String, i64> = (0..rng.below(3))
                .map(|i| (format!("k{i}{}", text(&mut rng)), int(&mut rng)))
                .collect();
            csv.push_str(&format!(
                "{},{n},{x:?},{ok},{}\n",
                cell(&name, &mut rng),
                dates::format(day)
            ));
            json.push(serde_json::json!({
                "name": name, "n": n, "x": x, "ok": ok, "day": dates::format(day), "tags": tags, "counts": counts,
            }));
            let base = vec![
                (names[1].clone(), Value::Date(day)),
                (names[2].clone(), Value::Int(n.into())),
                (names[3].clone(), Value::str(&name)),
                (names[4].clone(), Value::Bool(ok)),
                (names[6].clone(), Value::Float(x)),
            ];
            csv_rows.push(make_record(ty.clone(), base.clone()));
            let mut full = base;
            full.push((
                names[5].clone(),
                Value::list(tags.iter().map(|t| Value::str(t)).collect()),
            ));
            full.push((
                names[0].clone(),
                Value::map(
                    counts
                        .iter()
                        .map(|(k, v)| (Value::str(k), Value::Int((*v).into())))
                        .collect(),
                ),
            ));
            json_rows.push(make_record(ty.clone(), full));
        }
        let limits = InputLimits::default();
        let got = read(src_csv, csv.clone().into_bytes(), &limits)
            .unwrap_or_else(|e| panic!("case {case}: {}\n{csv}", e.message));
        assert_eq!(got, Value::list(csv_rows), "case {case}:\n{csv}");
        let text = serde_json::to_string(&json).unwrap();
        let got = read(src_json, text.clone().into_bytes(), &limits)
            .unwrap_or_else(|e| panic!("case {case}: {}\n{text}", e.message));
        assert_eq!(got, Value::list(json_rows), "case {case}:\n{text}");
        let ints: Vec<i64> = (0..rng.below(5)).map(|_| int(&mut rng)).collect();
        let lines: String = ints.iter().map(|n| format!("{n}\n")).collect();
        let got = read("let ns: list[int] = read(\"n.txt\")", lines.into_bytes(), &limits).unwrap();
        assert_eq!(
            got,
            Value::list(ints.into_iter().map(|n| Value::Int(n.into())).collect())
        );
    }
}

// ── Malformed data ───────────────────────────────────────────────────────

const SEEDS: &[(&str, &str, DataFormat)] = &[
    (
        "day,visitors,signups\n2026-09-01,250,9\n\"2026-09-02\",\"25,0\",6\n2026-09-03,250,\"1\"\"1\"\n",
        "type Day = { day: date, visitors: int, signups: int }\nlet rows: list[Day] = read(\"a.csv\")",
        DataFormat::Csv,
    ),
    (
        "name,share,size\n\"a, b\",30%,Small\nc,0.5,Large\n",
        "enum Size { Small, Large }\nlet rows: list[{ name: str, share: prob, size: Size }] = read(\"a.csv\")",
        DataFormat::Csv,
    ),
    (
        r#"{"price": 12.5, "churn": "4%", "segments": [{"name": "Small", "share": 0.7}, {"name": "Large", "share": 0.3}], "by_day": {"2027-01-01": [1, 2]}}"#,
        "enum Size { Small, Large }\ntype S = { name: Size, share: prob }\nlet a: { price: float, churn: prob, segments: list[S], by_day: map[date, list[int]] } = read(\"a.json\")",
        DataFormat::Json,
    ),
    (
        r#"[{"id": 1, "tags": ["a", "b"], "bag": {"x": 2}}, {"id": 2, "tags": [], "bag": ["y", "y"]}]"#,
        "type T = { id: int, tags: list[str], bag: bag[str] }\nlet a: list[T] = read(\"a.json\")",
        DataFormat::Json,
    ),
    (
        "1\n2\n\n3\n-4\n",
        "let a: list[int] = read(\"a.txt\")",
        DataFormat::Lines,
    ),
];

fn mutate(bytes: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = bytes.to_vec();
    let alphabet = b",\"\n\r{}[]:%-.0123456789eE \\\x00\xff\xc3tfn";
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(out.len() as u64 + 1) as usize;
        match rng.below(5) {
            0 if at < out.len() => {
                out.remove(at);
            }
            1 if at < out.len() => out[at] = *rng.pick(alphabet),
            2 => out.insert(at, *rng.pick(alphabet)),
            3 if !out.is_empty() => {
                // Repeat a chunk: long rows, deep nesting, many values.
                let from = rng.below(out.len() as u64) as usize;
                let to = (from + 1 + rng.below(40) as usize).min(out.len());
                let chunk = out[from..to].to_vec();
                for _ in 0..1 + rng.below(50) {
                    out.splice(at.min(out.len())..at.min(out.len()), chunk.iter().copied());
                }
            }
            _ => out.truncate(at),
        }
    }
    out
}

#[test]
fn malformed_data_never_panics() {
    let small = InputLimits {
        max_bytes: 4_000,
        max_values: 300,
        max_collection: 100,
        max_depth: 8,
        ..InputLimits::default()
    };
    let mut rng = Rng(7);
    let (mut read_ok, mut rejected) = (0, 0);
    for case in 0..cases() {
        let (data, src, format) = rng.pick(SEEDS);
        let bytes = mutate(data.as_bytes(), &mut rng);
        let limits = if rng.below(2) == 0 {
            &small
        } else {
            &InputLimits::default()
        };
        match read(src, bytes.clone(), limits) {
            Ok(_) => read_ok += 1,
            Err(e) => {
                assert_ne!(e.kind, ErrorKind::Internal, "case {case}: {}", e.message);
                rejected += 1;
            }
        }
        // The guesser reads the same bytes, within the same limits.
        let _ = suggest(&bytes, *format, "a", limits);
    }
    eprintln!("{read_ok} read, {rejected} rejected");
    assert!(read_ok > 0 && rejected > 0);
}
