//! Reading data: the rules that the compiler, the loader and `probl schema`
//! share (docs/data-input.md).

use crate::ir::{DataFormat, EnumType, RecordType, TypeSpec};
use probl_syntax::Span;

/// The key a name is matched by, between record fields and the columns or
/// keys of data: its letters and digits, in lowercase. `daily_sign_ups`,
/// `Daily sign-ups` and `dailySignUps` all have the key `dailysignups`.
pub fn field_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl DataFormat {
    pub fn name(self) -> &'static str {
        match self {
            DataFormat::Csv => "csv",
            DataFormat::Json => "json",
            DataFormat::Lines => "lines",
        }
    }

    pub fn from_name(name: &str) -> Option<DataFormat> {
        match name {
            "csv" => Some(DataFormat::Csv),
            "json" => Some(DataFormat::Json),
            "lines" => Some(DataFormat::Lines),
            _ => None,
        }
    }

    /// The format a path's extension implies. Standard input (`-`) is lines.
    pub fn from_path(path: &str) -> Option<DataFormat> {
        if path == "-" {
            return Some(DataFormat::Lines);
        }
        let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
        match extension.as_str() {
            "csv" => Some(DataFormat::Csv),
            "json" => Some(DataFormat::Json),
            "txt" => Some(DataFormat::Lines),
            _ => None,
        }
    }
}

/// A single value: a CSV cell, a line, or a JSON number, string or boolean.
pub fn is_plain(ty: &TypeSpec) -> bool {
    matches!(
        ty,
        TypeSpec::Int
            | TypeSpec::Float
            | TypeSpec::Prob
            | TypeSpec::Bool
            | TypeSpec::Str
            | TypeSpec::Date
            | TypeSpec::Enum(_)
    )
}

/// What a JSON object's keys, which are text, can be read as.
pub fn is_key(ty: &TypeSpec) -> bool {
    matches!(
        ty,
        TypeSpec::Int | TypeSpec::Bool | TypeSpec::Str | TypeSpec::Date | TypeSpec::Enum(_)
    )
}

/// Why data can't be read as a type.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub message: String,
    pub help: Option<String>,
    /// A record field's declaration, when the problem is there.
    pub at: Option<Span>,
}

impl Problem {
    fn new(message: impl Into<String>, help: Option<&str>, at: Option<Span>) -> Problem {
        Problem {
            message: message.into(),
            help: help.map(str::to_string),
            at,
        }
    }
}

/// Check that data in `format` can be read as `ty`: that `ty` holds only
/// values, fits the format, and has no two fields that would match the same
/// name in the data.
pub fn check(ty: &TypeSpec, format: DataFormat, records: &[RecordType], enums: &[EnumType]) -> Vec<Problem> {
    let mut checker = Checker {
        records,
        enums,
        seen: vec![false; records.len()],
        problems: Vec::new(),
    };
    let describe = |t: &TypeSpec| t.describe_in(records, enums);
    match format {
        DataFormat::Json => checker.value(ty, None),
        DataFormat::Csv => match ty {
            TypeSpec::List(row) if matches!(**row, TypeSpec::Record(_) | TypeSpec::AnonRecord(_)) => checker.row(row),
            _ => checker.problems.push(Problem::new(
                format!(
                    "a CSV file reads as a list of records, one per row, not as `{}`",
                    describe(ty)
                ),
                Some("declare a record type for the rows: `type Row = { … }`, then `let rows: list[Row] = read(…)`"),
                None,
            )),
        },
        DataFormat::Lines => match ty {
            TypeSpec::List(t) if is_plain(t) => {}
            _ => checker.problems.push(Problem::new(
                format!("lines read as a list of single values, not as `{}`", describe(ty)),
                Some("read one value per line, like `list[int]` or `list[str]`"),
                None,
            )),
        },
    }
    checker.problems
}

struct Checker<'a> {
    records: &'a [RecordType],
    enums: &'a [EnumType],
    /// Named record types already checked.
    seen: Vec<bool>,
    problems: Vec<Problem>,
}

impl Checker<'_> {
    fn describe(&self, ty: &TypeSpec) -> String {
        ty.describe_in(self.records, self.enums)
    }

    /// Any JSON value.
    fn value(&mut self, ty: &TypeSpec, at: Option<Span>) {
        match ty {
            _ if is_plain(ty) => {}
            TypeSpec::List(t) | TypeSpec::Bag(t) => self.value(t, at),
            TypeSpec::Map(k, v) => {
                if !is_key(k) {
                    self.problems.push(Problem::new(
                        format!(
                            "a map read from JSON has text keys, so they can't be `{}`",
                            self.describe(k)
                        ),
                        Some("a map's keys can be `str`, `int`, `bool`, `date` or an enum"),
                        at,
                    ));
                }
                self.value(v, at);
            }
            TypeSpec::Record(r) => {
                if std::mem::replace(&mut self.seen[*r as usize], true) {
                    return;
                }
                let fields: Vec<(String, TypeSpec, Option<Span>)> = self.records[*r as usize]
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone(), Some(f.span)))
                    .collect();
                self.distinct_names(&fields, at);
                for (_, t, span) in &fields {
                    self.value(t, span.or(at));
                }
            }
            TypeSpec::AnonRecord(fields) => {
                let fields: Vec<(String, TypeSpec, Option<Span>)> =
                    fields.iter().map(|(n, t)| (n.clone(), t.clone(), None)).collect();
                self.distinct_names(&fields, at);
                for (_, t, _) in &fields {
                    self.value(t, at);
                }
            }
            TypeSpec::Dist(_) => self.problems.push(Problem::new(
                format!("data can't be a distribution, like `{}`", self.describe(ty)),
                Some("read the values, then make a distribution of them, like `one_of(values)`"),
                at,
            )),
            _ => self.problems.push(Problem::new(
                format!("data can't be `{}`", self.describe(ty)),
                Some("data is numbers, text, dates, enums, and records, lists, maps and bags of those"),
                at,
            )),
        }
    }

    /// A CSV row: a record of single values.
    fn row(&mut self, ty: &TypeSpec) {
        let fields: Vec<(String, TypeSpec, Option<Span>)> = match ty {
            TypeSpec::Record(r) => self.records[*r as usize]
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ty.clone(), Some(f.span)))
                .collect(),
            TypeSpec::AnonRecord(fields) => fields.iter().map(|(n, t)| (n.clone(), t.clone(), None)).collect(),
            _ => return,
        };
        self.distinct_names(&fields, None);
        for (name, t, span) in &fields {
            if !is_plain(t) {
                self.problems.push(Problem::new(
                    format!(
                        "a CSV cell holds a single value, but `{name}` is a `{}`",
                        self.describe(t)
                    ),
                    Some("read the file as JSON, or give each value a column of its own"),
                    *span,
                ));
            }
        }
    }

    /// No two fields may match the same name in the data.
    fn distinct_names(&mut self, fields: &[(String, TypeSpec, Option<Span>)], at: Option<Span>) {
        for (i, (a, _, span)) in fields.iter().enumerate() {
            let key = field_key(a);
            if let Some((b, _, _)) = fields[..i].iter().find(|(b, _, _)| field_key(b) == key) {
                self.problems.push(Problem::new(
                    format!("the fields `{b}` and `{a}` would match the same names in the data"),
                    Some("names match ignoring case, spaces and punctuation: rename one of the fields"),
                    span.or(at),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_ignoring_case_and_punctuation() {
        for name in ["daily_sign_ups", "Daily sign-ups", "dailySignUps", "DAILY_SIGN_UPS"] {
            assert_eq!(field_key(name), "dailysignups");
        }
        // Other alphabets and digits count too, lowercased where they can be.
        assert_eq!(field_key("Größe (m²)"), "größem²");
        assert_ne!(field_key("a1"), field_key("a_2"));
    }

    #[test]
    fn formats_come_from_extensions() {
        assert_eq!(DataFormat::from_path("data/pilot.CSV"), Some(DataFormat::Csv));
        assert_eq!(DataFormat::from_path("a.json"), Some(DataFormat::Json));
        assert_eq!(DataFormat::from_path("counts.txt"), Some(DataFormat::Lines));
        assert_eq!(DataFormat::from_path("-"), Some(DataFormat::Lines));
        assert_eq!(DataFormat::from_path("sales.xlsx"), None);
        assert_eq!(DataFormat::from_path("README"), None);
    }
}
