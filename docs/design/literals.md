# Date, datetime and duration literals

> Implemented 29 October 2026. The [reference semantics](../semantics.md#1-values-and-types) is authoritative. This extends the original date-literal proposal with UTC offset normalization for `datetime`.

## Contract

Single quotes contain canonical values, never strings. The contents determine the type without an annotation:

```probl
let day: date = '2027-01-31'
let meeting: datetime = '2027-01-31T09:30:00'
let same_meeting = '2027-01-31T11:30:00+02:00'
report meeting == same_meeting  # true
report meeting.add_minutes(45) # 2027-01-31T10:15:00
```

Literals are validated during compilation, including in unreachable code. They work in comparisons, containers, record fields, function arguments and `match` patterns. They have no escapes or interpolation and must close on the same line. `'hello'` is an error with help directing text to double quotes.

A `date` is a Gregorian calendar day in years 1 through 9999, written exactly `YYYY-MM-DD`. A `datetime` adds `T` and `HH:MM:SS`, optionally followed by a decimal point and one to nine fractional-second digits. Hours are 00–23 and minutes/seconds 00–59. Leap seconds, named timezones, lowercase separators, a space in place of `T`, and omitted seconds are not supported.

## Offsets and storage

A datetime may end in uppercase `Z` or a numeric offset `±HH:MM`, with offset hours 00–23 and minutes 00–59. An explicit offset is subtracted to obtain UTC calendar fields, then discarded. For example, `'2027-01-01T00:15:00+01:00'` becomes `'2026-12-31T23:15:00'`. This uses [RFC 3339's numeric-offset convention](https://www.rfc-editor.org/rfc/rfc3339.html#section-4.2); this language's format is a restricted profile with optional offsets and at most nine fractional digits.

An unsuffixed value keeps its written fields. No machine timezone is consulted. `Z`, `+00:00` and `-00:00` all produce the same stored fields; original offset metadata is not retained. The supplied date and normalized result must both stay within years 1 through 9999. Normalization that crosses either endpoint is an error.

Storage is an integer day plus an integer nanosecond within that day. Comparisons, map/set identity, distribution outcomes and arithmetic retain all nine fractional digits. A datetime never silently becomes a date, number or string. A `date` and midnight on that date remain different values.

Printing omits offset suffixes and unnecessary trailing fractional zeroes. A scalar report and `str` use unquoted canonical text; nested values use single quotes so lists, maps and records can be written back as source. `today` remains a date supplied by the host once per run; `now` supplies the associated datetime snapshot without reading the clock during execution.

## Construction and parsing

A type's name builds from its parts:

```probl
let day = date(2027, 1, 31)
let meeting = datetime(2027, 1, 31, 9, 30, 0)
let precise = datetime(2027, 1, 31, 9, 30, 0, 123456789)
```

`date(datetime)` drops normalized clock fields; `datetime(date)` chooses midnight. Both preserve inputs already of the target type and lift over finite distributions. The optional seventh datetime component is nanoseconds, defaulting to zero. Components must be ints. `date(text)` is removed; its diagnostic explains the literal and runtime parser alternatives.

Runtime text is explicit:

```probl
let start = parse_date(env.get("START", "2027-01-31"))
let meeting = parse_datetime(env.get("MEETING", "2027-01-31T09:30:00Z"))
```

The parsers share literal validation and offset normalization. Invalid text produces a catchable `ConversionError` at runtime. Input schemas accept `datetime` for CSV cells, lines, JSON strings and map keys; schema inference recognizes it. Inputs permit their usual surrounding whitespace, while literals and runtime parsers do not.

## Operations

- `year`, `month`, `day`, `weekday` and `is_workday` accept dates and datetimes.
- `add_days`, `add_months`, `add_years`, `add_workdays`, `start_of_month` and `end_of_month` preserve the input kind and any clock fields. Month/year shifts clamp the day as for dates. Holidays remain a list of dates.
- `hour`, `minute`, `second` and `nanosecond` extract clock components.
- `add_hours`, `add_minutes`, `add_seconds` and `add_nanoseconds` shift a datetime by an integer count, carrying across calendar boundaries. Each day is exactly 24 hours; no daylight-saving transitions apply.
- Datetime arithmetic also accepts typed durations; subtraction of datetimes produces an exact duration. Bare numeric offsets are rejected, and out-of-range or fractional-nanosecond results fail.
- Subtraction of dates also produces an exact duration, with each calendar day represented as 24 hours. Divide by `'1d'` to obtain a numeric day count.
- Ordering, extrema, CDFs, quantiles and low/high medians work on homogeneous datetime populations. Midpoint medians round half a nanosecond toward the earlier value. Datetime means round once to the nearest nanosecond, ties earlier. Standard deviation and variance use the unrounded mean and return duration and squared_duration.
- Reports retain exact datetime quantiles and medians, including typed canonical strings in JSON. The playground keeps datetime charts in the text view rather than converting nanoseconds through JavaScript numbers.

The constructors, parsers and transformations lift over finite distributions, as the date APIs do.

## Migration and validation

Constant `date("YYYY-MM-DD")` calls become single-quoted literals; dynamic text uses `parse_date`. Existing date constructors with three integer components are unchanged. Nested date displays now include single quotes.

Tests cover leap years and year endpoints; malformed literals in unreachable branches; fractional precision; positive, negative and zero offsets; normalization across days, months and years; constructor/parser agreement; first-class functions; unions and typed containers; match patterns; exact collection identity; calendar/clock arithmetic; input schemas; report JSON; and nested display round trips. Playground examples and native/WASM fixtures use the new syntax.

## Deferred literals

Times of day and timezone-preserving instants remain separate designs. They may use distinct single-quoted shapes, or a prefix if a new shape would conflict. Date, datetime and duration literals never depend on the receiving annotation to select their type.

## Duration literals and statistics

Durations use compact unit-bearing literals such as `'1h 30m'`, `'-0.5ns'` and `'0.25s^2'`. The last has type `squared_duration`, used for variances. Supported units are `w`, `d`, `h`, `m`, `s`, `ms`, `us`, `ns`; days and weeks have fixed lengths. A leading minus applies to all components. Equivalent units normalize to the same value. Whole base units remain exact integers; fractional base units use finite binary64. Printing chooses a compact unit that round-trips, with single quotes inside containers.

`days` and `weeks` now build durations without rounding, alongside the hour-through-nanosecond constructors. `parse_duration` handles runtime text. Adding to a datetime requires whole nanoseconds; adding to a date requires whole 24-hour days. Multiplication, division and square roots check dimensions. Explicit ratios such as `sd(times) / '1ms'` and `variance(times) / '1s^2'` produce numbers whose unit the expression states.

Datetime moment calculations retain exact integer coordinates and exact stored binary64 weights until final rounding. Lists use unit weights before any duplicate merging. SD and variance are population spread, not sampling error, and preserve time dimensions. JSON exposes typed `mean_value` and `sd_value`; existing numeric estimate fields remain null for time populations. Exact report retention supports these statistics; the numeric-only moments and sketch collection policies remain separate.
