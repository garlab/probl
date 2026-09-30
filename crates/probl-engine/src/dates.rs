//! Immutable Gregorian dates, stored as days since 1970-01-01.
//! Public domain: 0001-01-01 through 9999-12-31. No clock is read here.

pub const MIN: i32 = -719_162;
pub const MAX: i32 = 2_932_896;

pub fn valid(days: i32) -> bool {
    (MIN..=MAX).contains(&days)
}

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

pub fn from_parts(y: i64, m: i64, d: i64) -> Option<i32> {
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m as u32) as i64 {
        return None;
    }
    Some(from_civil(y, m as u32, d as u32) as i32)
}

/// Exactly ten ASCII bytes. Validate before doing any calendar arithmetic.
pub fn parse(text: &str) -> Option<i32> {
    let b = text.as_bytes();
    if b.len() != 10
        || b[4] != b'-'
        || b[7] != b'-'
        || !b
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    from_parts(
        text[..4].parse().ok()?,
        text[5..7].parse().ok()?,
        text[8..].parse().ok()?,
    )
}

pub fn add_days(days: i32, n: i64) -> Option<i32> {
    let result = (days as i64).checked_add(n)?;
    (valid(days) && (MIN as i64..=MAX as i64).contains(&result)).then_some(result as i32)
}

pub fn from_unix_seconds(seconds: i64) -> Option<i32> {
    let days = i32::try_from(seconds.div_euclid(86_400)).ok()?;
    valid(days).then_some(days)
}

pub fn add_months(days: i32, n: i64) -> Option<i32> {
    if !valid(days) {
        return None;
    }
    let (y, m, d) = to_civil(days as i64);
    let month = (y as i128 - 1) * 12 + m as i128 - 1 + n as i128;
    if !(0..9999 * 12).contains(&month) {
        return None;
    }
    let (y, m) = ((month / 12 + 1) as i64, (month % 12 + 1) as u32);
    from_parts(y, m as i64, d.min(days_in_month(y, m)) as i64)
}

pub fn add_years(days: i32, n: i64) -> Option<i32> {
    add_months(days, n.checked_mul(12)?)
}

pub fn start_of_month(days: i32) -> Option<i32> {
    if !valid(days) {
        return None;
    }
    let (y, m, _) = to_civil(days as i64);
    from_parts(y, m as i64, 1)
}

pub fn end_of_month(days: i32) -> Option<i32> {
    if !valid(days) {
        return None;
    }
    let (y, m, _) = to_civil(days as i64);
    from_parts(y, m as i64, days_in_month(y, m) as i64)
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

/// Weekdays only, in constant time even for huge offsets. The starting date
/// is not counted. A zero offset leaves even a weekend unchanged.
pub fn add_workdays(days: i32, n: i64) -> Option<i32> {
    if !valid(days) {
        return None;
    }
    let mut d = days as i128;
    let mut w = weekday(days) as i128;
    let mut left = n.unsigned_abs() as i128;
    let step = if n >= 0 { 1 } else { -1 };
    if left > 0 && w >= 5 {
        d += if step > 0 { 7 - w } else { 4 - w };
        w = if step > 0 { 0 } else { 4 };
        left -= 1;
    }
    let extra = left % 5;
    let weekend = if (step > 0 && w + extra >= 5) || (step < 0 && extra > w) {
        2
    } else {
        0
    };
    if left > 0 {
        d += step * (left / 5 * 7 + extra + weekend);
    }
    (MIN as i128..=MAX as i128).contains(&d).then_some(d as i32)
}

/// Holidays must be sorted, unique and weekdays. Each crossed holiday is
/// visited once, extending the target by one weekday in the chosen direction.
pub fn add_workdays_with_holidays(days: i32, n: i64, holidays: &[i32]) -> Option<i32> {
    let mut target = add_workdays(days, n)?;
    if n > 0 {
        for &h in &holidays[holidays.partition_point(|h| *h <= days)..] {
            if h > target {
                break;
            }
            target = add_workdays(target, 1)?;
        }
    } else if n < 0 {
        for &h in holidays[..holidays.partition_point(|h| *h < days)].iter().rev() {
            if h < target {
                break;
            }
            target = add_workdays(target, -1)?;
        }
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_round_trips_and_parsing_is_strict() {
        assert_eq!(from_parts(1, 1, 1), Some(MIN));
        assert_eq!(from_parts(9999, 12, 31), Some(MAX));
        for d in from_civil(1800, 1, 1)..from_civil(2200, 1, 1) {
            let (y, m, day) = to_civil(d);
            assert_eq!(from_parts(y, m as i64, day as i64), Some(d as i32));
            assert_eq!(parse(&format(d as i32)), Some(d as i32));
        }
        for s in [
            "2026-02-29",
            "1900-02-29",
            "0000-01-01",
            "10000-01-01",
            "2026-1-01",
            "+026-01-01",
            "2026-01-1",
            "2026-01-01 ",
            "2026-01-01T00:00:00Z",
            "🙂-01-01",
            "9223372036854775807-01-01",
        ] {
            assert_eq!(parse(s), None, "{s}");
        }
        assert!(parse("2000-02-29").is_some());
        assert_eq!(from_unix_seconds(-1), parse("1969-12-31"));
        assert_eq!(from_unix_seconds(0), parse("1970-01-01"));
        assert_eq!(from_unix_seconds(i64::MAX), None);
    }

    #[test]
    fn workday_jumps_match_a_daily_walk_in_both_directions() {
        let base = parse("2026-09-28").unwrap();
        let holidays = [base + 1, base + 4, base + 7, base + 8, base + 9, base + 10, base + 11];
        for start in base - 14..=base + 14 {
            for n in -40i64..=40 {
                for calendar in [&[][..], &holidays[..]] {
                    let mut expected = start;
                    let mut left = n.unsigned_abs();
                    while left != 0 {
                        expected += n.signum() as i32;
                        if weekday(expected) < 5 && !calendar.contains(&expected) {
                            left -= 1;
                        }
                    }
                    assert_eq!(
                        add_workdays_with_holidays(start, n, calendar),
                        Some(expected),
                        "{start}, {n}"
                    );
                }
            }
        }
        for n in [i64::MIN, i64::MAX] {
            assert_eq!(add_workdays(base, n), None);
        }
        assert_eq!(add_workdays(MAX, 1), None);
        assert_eq!(add_workdays(MIN, -1), None);
    }

    #[test]
    fn month_and_year_shifts_clamp_and_check_bounds() {
        let jan = parse("2024-01-31").unwrap();
        assert_eq!(add_months(jan, 1), parse("2024-02-29"));
        assert_eq!(add_months(jan, 2), parse("2024-03-31"));
        assert_eq!(add_months(jan, -1), parse("2023-12-31"));
        assert_eq!(add_years(parse("2024-02-29").unwrap(), 1), parse("2025-02-28"));
        assert_eq!(end_of_month(MAX), Some(MAX));
        assert_eq!(start_of_month(MIN), Some(MIN));
        for n in [i64::MIN, i64::MAX] {
            assert_eq!(add_months(jan, n), None);
            assert_eq!(add_years(jan, n), None);
            assert_eq!(add_days(jan, n), None);
        }
        assert_eq!(add_months(MIN, -1), None);
        assert_eq!(add_years(MAX, 1), None);
    }
}
