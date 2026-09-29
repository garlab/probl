//! `probl schema FILE`: a type to read a file with, guessed from what's in
//! it (docs/data-input.md). It's a suggestion for the program's author to
//! paste and edit: running a program never guesses. It reads the file
//! within the same limits as loading it.

use super::InputLimits;
use crate::dates;
use crate::report::thousands;
use csv::{ReaderBuilder, StringRecord, Trim};
use probl_sema::data::field_key;
use probl_sema::ir::DataFormat;
use probl_syntax::token::is_name;

/// Suggest a declaration, and the `read` that uses it, for the data in
/// `bytes`, read from `file` in `format`.
pub fn suggest(bytes: &[u8], format: DataFormat, file: &str, limits: &InputLimits) -> Result<String, String> {
    if bytes.len() as u64 > limits.max_bytes {
        return Err(format!(
            "the file is more than {} MiB",
            limits.max_bytes.div_ceil(1024 * 1024)
        ));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let stem = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
    let fallback = if format == DataFormat::Csv { "Row" } else { "Data" };
    let type_name = camel(stem).unwrap_or_else(|| fallback.to_string());
    let var = field_name(stem).unwrap_or_else(|| "data".to_string());
    let mut out = Output {
        notes: Vec::new(),
        values: 0,
        limits,
    };
    let text = match format {
        DataFormat::Csv => {
            let fields = csv_fields(bytes, &mut out)?;
            format!(
                "type {type_name} = {{ {} }}\n# let {var}: list[{type_name}] = read({file:?})\n",
                fields.join(", ")
            )
        }
        DataFormat::Lines => {
            let text =
                std::str::from_utf8(bytes).map_err(|_| "the file isn't text: it isn't valid UTF-8".to_string())?;
            let mut column = Column::default();
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                out.count()?;
                column.add(line, out.limits.max_integer_bits);
            }
            format!("# let {var}: list[{}] = read({file:?})\n", column.guess())
        }
        DataFormat::Json => {
            let value: serde_json::Value =
                serde_json::from_slice(bytes).map_err(|e| format!("this isn't valid JSON: {e}"))?;
            let shape = shape(&value, "", &mut out)?;
            match &shape {
                Shape::Record(_) => format!(
                    "type {type_name} = {}\n# let {var}: {type_name} = read({file:?})\n",
                    render(&shape, &mut out, "")
                ),
                Shape::List(item) if matches!(**item, Shape::Record(_)) => format!(
                    "type {type_name} = {}\n# let {var}: list[{type_name}] = read({file:?})\n",
                    render(item, &mut out, "")
                ),
                _ => format!("# let {var}: {} = read({file:?})\n", render(&shape, &mut out, "")),
            }
        }
    };
    let mut notes = out.notes;
    notes.dedup();
    Ok(notes.iter().map(|n| format!("# {n}\n")).collect::<String>() + &text)
}

/// Notes for the reader, and the values counted against the limit.
struct Output<'a> {
    notes: Vec<String>,
    values: u64,
    limits: &'a InputLimits,
}

impl Output<'_> {
    fn count(&mut self) -> Result<(), String> {
        self.values += 1;
        if self.values > self.limits.max_values {
            return Err(format!(
                "the file has more than {} values",
                thousands(self.limits.max_values.min(i64::MAX as u64) as i64)
            ));
        }
        Ok(())
    }
}

/// A field name for a column or key, matching it, if there is one.
fn field_name(name: &str) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in name.chars() {
        if c.is_alphanumeric() {
            if c.is_uppercase() && previous_lower && !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = c.is_lowercase() || c.is_ascii_digit();
            word.extend(c.to_lowercase());
        } else {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            previous_lower = false;
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    let snake = words.join("_");
    [snake.clone(), format!("{snake}_")]
        .into_iter()
        .find(|n| is_name(n) && field_key(n) == field_key(name))
}

/// A type name for a file name: `sales-2026` gives `Sales2026`.
fn camel(stem: &str) -> Option<String> {
    let name: String = stem
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            let first = cs.next().unwrap().to_ascii_uppercase();
            std::iter::once(first).chain(cs).collect::<String>()
        })
        .collect();
    is_name(&name).then_some(name)
}

/// What a column of text could be: the first of these types that reads
/// every value, ignoring empty ones.
#[derive(Clone, Copy, Debug)]
struct Column {
    int: bool,
    float: bool,
    prob: bool,
    bool_: bool,
    date: bool,
    values: bool,
    empty: bool,
}

impl Default for Column {
    fn default() -> Column {
        Column {
            int: true,
            float: true,
            prob: true,
            bool_: true,
            date: true,
            values: false,
            empty: false,
        }
    }
}

impl Column {
    fn add(&mut self, text: &str, max_integer_bits: u64) {
        let t = text.trim();
        if t.is_empty() {
            self.empty = true;
            return;
        }
        self.values = true;
        self.int &= t
            .parse::<probl_number::Integer>()
            .is_ok_and(|n| n.bits() <= max_integer_bits);
        self.float &= t.parse::<f64>().is_ok_and(f64::is_finite);
        self.prob &= t
            .strip_suffix('%')
            .and_then(|p| p.trim().parse::<f64>().ok())
            .is_some_and(|x| (0.0..=100.0).contains(&x));
        self.bool_ &= t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("false");
        self.date &= dates::parse(t).is_some();
    }

    fn guess(&self) -> &'static str {
        match self {
            _ if !self.values => "str",
            Column { int: true, .. } => "int",
            Column { float: true, .. } => "float",
            Column { prob: true, .. } => "prob",
            Column { bool_: true, .. } => "bool",
            Column { date: true, .. } => "date",
            _ => "str",
        }
    }
}

fn csv_fields(bytes: &[u8], out: &mut Output) -> Result<Vec<String>, String> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .trim(Trim::None)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|e| format!("can't read it as CSV: {e}"))?
        .clone();
    if headers.is_empty() {
        return Err("the file is empty: a CSV file starts with a row of column names".to_string());
    }
    let mut columns = vec![Column::default(); headers.len()];
    let mut record = StringRecord::new();
    while reader
        .read_record(&mut record)
        .map_err(|e| format!("can't read it as CSV: {e}"))?
    {
        for (column, cell) in columns.iter_mut().zip(record.iter()) {
            out.count()?;
            column.add(cell, out.limits.max_integer_bits);
        }
    }
    let mut fields = Vec::new();
    let mut keys: Vec<(String, String)> = Vec::new();
    for (header, column) in headers.iter().zip(&columns) {
        let Some(name) = field_name(header) else {
            out.notes.push(format!(
                "the column {header:?} can't be a field name: rename it in the file to read it"
            ));
            continue;
        };
        if let Some((other, _)) = keys.iter().find(|(_, k)| *k == field_key(header)) {
            out.notes.push(format!(
                "the columns {other:?} and {header:?} would match the same field: rename one to read either"
            ));
            continue;
        }
        keys.push((header.to_string(), field_key(header)));
        let ty = column.guess();
        if column.empty && ty != "str" {
            out.notes.push(format!(
                "`{name}` has empty cells, which only a `str` field reads for now"
            ));
        }
        fields.push(format!("{name}: {ty}"));
    }
    Ok(fields)
}

/// The type a JSON value suggests.
#[derive(Clone, Debug, PartialEq)]
enum Shape {
    /// Nothing to go on: an empty array, or only nulls.
    Unknown,
    Int,
    Float,
    Prob,
    Bool,
    Date,
    Str,
    List(Box<Shape>),
    Map(&'static str, Box<Shape>),
    Record(Vec<(String, Shape)>),
    /// Values of different kinds.
    Mixed,
}

fn shape(value: &serde_json::Value, at: &str, out: &mut Output) -> Result<Shape, String> {
    use serde_json::Value as J;
    out.count()?;
    Ok(match value {
        J::Null => {
            out.notes.push(format!(
                "{} has nulls: the language has no missing values yet",
                place(at)
            ));
            Shape::Unknown
        }
        J::Bool(_) => Shape::Bool,
        J::Number(n) => {
            let text = n.to_string();
            if !text.contains(['.', 'e', 'E']) {
                let n = text.parse::<probl_number::Integer>().map_err(|e| e.to_string())?;
                if n.bits() > out.limits.max_integer_bits {
                    return Err("integer size exceeds the input limit".into());
                }
                Shape::Int
            } else {
                Shape::Float
            }
        }
        J::String(s) => {
            let mut column = Column::default();
            column.add(s, out.limits.max_integer_bits);
            match column.guess() {
                "date" => Shape::Date,
                "prob" => Shape::Prob,
                _ => Shape::Str,
            }
        }
        J::Array(items) => {
            let mut item = Shape::Unknown;
            for (i, x) in items.iter().enumerate() {
                item = unify(item, shape(x, &format!("{at}[{i}]"), out)?);
            }
            Shape::List(Box::new(item))
        }
        J::Object(entries) => {
            // Keys that are all dates or all integers make a map.
            let key_type = if entries.is_empty() {
                None
            } else if entries.keys().all(|k| dates::parse(k).is_some()) {
                Some("date")
            } else if entries.keys().all(|k| {
                k.parse::<probl_number::Integer>()
                    .is_ok_and(|n| n.bits() <= out.limits.max_integer_bits)
            }) {
                Some("int")
            } else {
                None
            };
            match key_type {
                Some(k) => {
                    let mut value = Shape::Unknown;
                    for (key, x) in entries {
                        value = unify(value, shape(x, &format!("{at}.{key}"), out)?);
                    }
                    Shape::Map(k, Box::new(value))
                }
                None => {
                    let mut fields = Vec::new();
                    for (key, x) in entries {
                        let path = if at.is_empty() {
                            key.clone()
                        } else {
                            format!("{at}.{key}")
                        };
                        fields.push((key.clone(), shape(x, &path, out)?));
                    }
                    Shape::Record(fields)
                }
            }
        }
    })
}

fn place(at: &str) -> String {
    if at.is_empty() {
        "the document".to_string()
    } else {
        format!("`{at}`")
    }
}

fn unify(a: Shape, b: Shape) -> Shape {
    use Shape::*;
    match (a, b) {
        (Unknown, x) | (x, Unknown) => x,
        (a, b) if a == b => a,
        (Int, Float) | (Float, Int) => Float,
        (Date | Prob | Str, Date | Prob | Str) => Str,
        (List(a), List(b)) => List(Box::new(unify(*a, *b))),
        (Map(k, a), Map(l, b)) if k == l => Map(k, Box::new(unify(*a, *b))),
        (Record(a), Record(b)) => {
            let mut fields = a;
            for (name, shape) in b {
                match fields.iter_mut().find(|(n, _)| *n == name) {
                    Some((_, s)) => *s = unify(std::mem::replace(s, Unknown), shape),
                    None => fields.push((name, shape)),
                }
            }
            Record(fields)
        }
        _ => Mixed,
    }
}

fn render(shape: &Shape, out: &mut Output, at: &str) -> String {
    match shape {
        Shape::Unknown => {
            out.notes
                .push(format!("{} gives nothing to guess from: `str` is a guess", place(at)));
            "str".into()
        }
        Shape::Int => "int".into(),
        Shape::Float => "float".into(),
        Shape::Prob => "prob".into(),
        Shape::Bool => "bool".into(),
        Shape::Date => "date".into(),
        Shape::Str => "str".into(),
        Shape::Mixed => {
            out.notes.push(format!(
                "{} mixes different kinds of values: `str` is a guess",
                place(at)
            ));
            "str".into()
        }
        Shape::List(item) => format!("list[{}]", render(item, out, &format!("{at}[]"))),
        Shape::Map(k, v) => format!("map[{k}, {}]", render(v, out, &format!("{at}.*"))),
        Shape::Record(fields) => render_fields(fields, out, at),
    }
}

fn render_fields(fields: &[(String, Shape)], out: &mut Output, at: &str) -> String {
    let mut parts = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for (key, shape) in fields {
        let path = if at.is_empty() {
            key.clone()
        } else {
            format!("{at}.{key}")
        };
        let Some(name) = field_name(key) else {
            out.notes
                .push(format!("the key {key:?} can't be a field name: it's left out"));
            continue;
        };
        if keys.contains(&field_key(key)) {
            out.notes.push(format!(
                "the key {key:?} would match the same field as another: it's left out"
            ));
            continue;
        }
        keys.push(field_key(key));
        parts.push(format!("{name}: {}", render(shape, out, &path)));
    }
    format!("{{ {} }}", parts.join(", "))
}
