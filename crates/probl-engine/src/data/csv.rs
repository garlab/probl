//! CSV files, as lists of records (RFC 4180, with a header row). The `csv`
//! crate splits the cells; the declared type says what each one is.

use super::{Cx, Problem, quoted, text};
use crate::ops;
use crate::value::Value;
use csv::{ErrorKind, ReaderBuilder, StringRecord, Trim};
use probl_sema::data::field_key;
use probl_sema::ir::TypeSpec;
use std::sync::Arc;

pub(crate) fn read(bytes: &[u8], ty: &TypeSpec, cx: &mut Cx) -> Result<Value, Problem> {
    let TypeSpec::List(row) = ty else {
        return Err(Problem::new("a CSV file reads as a list of records"));
    };
    let (type_name, fields): (Option<Arc<str>>, Vec<(String, TypeSpec)>) = match &**row {
        TypeSpec::Record(r) => {
            let decl = &cx.program.records[*r as usize];
            let fields = decl.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect();
            (Some(cx.name(&decl.name.clone())), fields)
        }
        TypeSpec::AnonRecord(fields) => (None, fields.clone()),
        _ => return Err(Problem::new("a CSV file reads as a list of records")),
    };
    if let Some(line) = unclosed_quote(bytes) {
        return Err(Problem::new("a quote isn't closed")
            .at(format!("line {line}"))
            .help("a cell in quotes ends with another quote; a quote inside it is written twice, like \"\""));
    }
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .flexible(false)
        .trim(Trim::None)
        .from_reader(bytes);
    let headers = reader.headers().map_err(problem)?.clone();
    // The column of each field.
    let mut columns = Vec::with_capacity(fields.len());
    for (name, _) in &fields {
        let key = field_key(name);
        let matching: Vec<usize> = (0..headers.len()).filter(|&i| field_key(&headers[i]) == key).collect();
        match matching[..] {
            [i] => columns.push(i),
            [] => {
                return Err(Problem::new(format!("no column matches the field `{name}`"))
                    .note(format!("the columns are {}", names(&headers)))
                    .help("names match ignoring case, spaces and punctuation"));
            }
            [a, b, ..] => {
                return Err(Problem::new(format!(
                    "the columns {} and {} both match the field `{name}`",
                    quoted(&headers[a]),
                    quoted(&headers[b])
                ))
                .help("names match ignoring case, spaces and punctuation: rename one of the columns"));
            }
        }
    }
    let field_names: Vec<Arc<str>> = fields.iter().map(|(n, _)| cx.name(n)).collect();
    let mut rows = Vec::new();
    let mut record = StringRecord::new();
    while reader.read_record(&mut record).map_err(problem)? {
        let line = record.position().map_or(0, |p| p.line());
        cx.budget.collection(rows.len() + 1)?;
        cx.budget.value()?;
        let mut values = Vec::with_capacity(fields.len());
        for (i, (_, ty)) in fields.iter().enumerate() {
            cx.budget.value()?;
            let column = columns[i];
            let v = text::plain(&record[column], ty, cx.program, &mut cx.budget)
                .map_err(|p| p.at(format!("line {line}, column {}", quoted(&headers[column]))))?;
            values.push((field_names[i].clone(), v));
        }
        rows.push(ops::make_record(type_name.clone(), values));
    }
    Ok(Value::list(rows))
}

/// The header's names, for messages.
fn names(headers: &StringRecord) -> String {
    let names: Vec<String> = headers.iter().map(quoted).collect();
    if names.is_empty() {
        "none: the file is empty".to_string()
    } else {
        names.join(", ")
    }
}

fn problem(e: csv::Error) -> Problem {
    match e.kind() {
        ErrorKind::UnequalLengths { pos, expected_len, len } => {
            let p = Problem::new(format!(
                "this row has {len} cell{}, but the header has {expected_len}",
                if *len == 1 { "" } else { "s" }
            ))
            .help("every row has as many cells as the header; a cell with a comma in it goes in quotes");
            match pos {
                Some(pos) => p.at(format!("line {}", pos.line())),
                None => p,
            }
        }
        ErrorKind::Utf8 { pos, .. } => {
            let p = Problem::new("this isn't text: it isn't valid UTF-8");
            match pos {
                Some(pos) => p.at(format!("line {}", pos.line())),
                None => p,
            }
        }
        _ => Problem::new(format!("can't read it as CSV: {e}")),
    }
}

/// The line where a quoted cell starts that isn't closed, if there is one:
/// the `csv` crate would take the rest of the file as that cell.
fn unclosed_quote(bytes: &[u8]) -> Option<u64> {
    let (mut line, mut opened) = (1, 0);
    let (mut field_start, mut in_quotes) = (true, false);
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if in_quotes {
            match b {
                b'"' if bytes.get(i) == Some(&b'"') => i += 1,
                b'"' => in_quotes = false,
                b'\n' => line += 1,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' if field_start => {
                in_quotes = true;
                opened = line;
            }
            b',' | b'\r' => {
                field_start = true;
                continue;
            }
            b'\n' => {
                line += 1;
                field_start = true;
                continue;
            }
            _ => {}
        }
        field_start = false;
    }
    in_quotes.then_some(opened)
}
