//! Calendar dates as days since 1970-01-01 (proleptic Gregorian calendar).

/// Days since 1970-01-01 for a year, month (1–12) and day (1–31).
pub fn from_civil(y: i64, m: u32, d: u32) -> i64 {
    // Howard Hinnant's days_from_civil.
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Year, month and day of a day number.
pub fn to_civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub fn format(days: i32) -> String {
    let (y, m, d) = to_civil(days as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Parse `YYYY-MM-DD`.
pub fn parse(text: &str) -> Option<i32> {
    let mut parts = text.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
        return None;
    }
    i32::try_from(from_civil(y, m, d)).ok()
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(days: i32) -> u32 {
    // 1970-01-01 was a Thursday.
    ((days as i64 + 3).rem_euclid(7)) as u32
}

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Add `n` working days (Monday to Friday); negative `n` goes back.
pub fn add_workdays(days: i32, n: i64) -> i32 {
    let mut d = days;
    let step = if n >= 0 { 1 } else { -1 };
    let mut left = n.abs();
    while left > 0 {
        d += step;
        if weekday(d) < 5 {
            left -= 1;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for &(y, m, d) in &[(1970, 1, 1), (2000, 2, 29), (2026, 9, 28), (1969, 12, 31), (2400, 3, 1)] {
            let days = from_civil(y, m, d);
            assert_eq!(to_civil(days), (y, m, d));
        }
        assert_eq!(from_civil(1970, 1, 1), 0);
        assert_eq!(format(parse("2026-09-28").unwrap()), "2026-09-28");
        assert_eq!(parse("2026-02-29"), None);
    }

    #[test]
    fn weekdays_and_workdays() {
        let monday = parse("2026-09-28").unwrap();
        assert_eq!(WEEKDAYS[weekday(monday) as usize], "Monday");
        assert_eq!(format(add_workdays(monday, 5)), "2026-10-05");
        assert_eq!(format(add_workdays(monday, -1)), "2026-09-25");
    }
}
