//! UTF-8 text construction with checks before allocation. String positions
//! elsewhere in the language count Unicode scalar values, never bytes.

use crate::dist::Budget;
use crate::error::{OpError, OpResult};
use crate::value::Value;
use std::fmt::{self, Write};

pub fn value(s: &str, budget: &mut Budget) -> OpResult<Value> {
    budget.string_work(s)?;
    budget.string_allocation(s.len())?;
    Ok(Value::str(s))
}

pub fn push(out: &mut String, s: &str, budget: &mut Budget) -> OpResult<()> {
    let size = out
        .len()
        .checked_add(s.len())
        .ok_or_else(|| OpError::limit("string size overflow"))?;
    budget.string_size(size)?;
    budget.string_work(s)?;
    budget.string_allocation(s.len())?;
    out.push_str(s);
    Ok(())
}

/// Format incrementally, so even a nested collection cannot allocate an
/// unbounded temporary through `to_string()` before the limit is checked.
pub fn push_value(out: &mut String, value: &Value, budget: &mut Budget) -> OpResult<()> {
    if crate::analytic::contains(value) {
        return Err(crate::analytic::unsupported("formatting an analytic outcome as text"));
    }
    struct Writer<'a> {
        out: &'a mut String,
        budget: &'a mut Budget,
        error: Option<OpError>,
    }
    impl Write for Writer<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            push(self.out, s, self.budget).map_err(|e| {
                self.error = Some(e);
                fmt::Error
            })
        }
    }
    let mut writer = Writer {
        out,
        budget,
        error: None,
    };
    write!(writer, "{value}").map_err(|_| writer.error.expect("only budget failures stop formatting"))
}

pub fn formatted(value: &Value, budget: &mut Budget) -> OpResult<Value> {
    let mut out = String::new();
    push_value(&mut out, value, budget)?;
    Ok(Value::str(&out))
}

pub fn chars(s: &str, budget: &mut Budget) -> OpResult<Vec<Value>> {
    budget.string_work(s)?;
    let count = s.chars().count();
    budget.collection(count as u128)?;
    budget.work(count as u64)?;
    budget.string_allocation(s.len())?;
    Ok(s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect())
}

pub fn case(s: &str, upper: bool, budget: &mut Budget) -> OpResult<Value> {
    budget.string_work(s)?;
    // Unicode's context-sensitive final sigma changes the scalar, but not its
    // UTF-8 length. Count scalar mappings first, then use str's full mapping.
    let bytes: usize = if upper {
        s.chars().flat_map(char::to_uppercase).map(char::len_utf8).sum()
    } else {
        s.chars().flat_map(char::to_lowercase).map(char::len_utf8).sum()
    };
    budget.string_allocation(bytes)?;
    let out = if upper { s.to_uppercase() } else { s.to_lowercase() };
    debug_assert_eq!(out.len(), bytes);
    Ok(Value::str(&out))
}
