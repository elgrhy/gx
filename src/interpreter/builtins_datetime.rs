//! Date/time builtins — production-grade temporal operations using chrono.
//! All functions accept Unix timestamps (f64 seconds) or ISO-8601 strings interchangeably.

use super::Signal;
use crate::value::Value;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc};
use std::collections::HashMap;

// ── Public entry points ───────────────────────────────────────────────────────

/// date_now() → ISO-8601 string of current UTC time
pub fn date_now_impl() -> Result<Value, Signal> {
    Ok(Value::Str(Utc::now().to_rfc3339()))
}

/// date_timestamp() → Unix timestamp as number (seconds since epoch)
pub fn date_timestamp_impl() -> Result<Value, Signal> {
    Ok(Value::Number(Utc::now().timestamp() as f64))
}

/// date_parse(string, format?) → Unix timestamp (number)
/// Auto-detects ISO-8601, RFC 2822, and common date formats when no format given.
pub fn date_parse_impl(args: &[Value]) -> Result<Value, Signal> {
    let s = args
        .first()
        .and_then(|v| v.as_str().map(String::from))
        .ok_or_else(|| Signal::Error("date_parse(string, format?)".into()))?;

    let fmt = args.get(1).and_then(|v| v.as_str().map(String::from));

    if let Some(ref fmt) = fmt {
        return parse_with_format(&s, fmt);
    }

    // RFC3339 / ISO-8601 with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(&s) {
        return Ok(Value::Number(dt.timestamp() as f64));
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(&s) {
        return Ok(Value::Number(dt.timestamp() as f64));
    }

    // Try common formats (datetime first, then date-only)
    let datetime_fmts = [
        "%Y-%m-%dT%H:%M:%SZ",
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%d/%m/%Y %H:%M:%S",
        "%m/%d/%Y %H:%M:%S",
    ];
    let date_fmts = [
        "%Y-%m-%d",
        "%d/%m/%Y",
        "%m/%d/%Y",
        "%d-%m-%Y",
        "%Y/%m/%d",
        "%B %d, %Y",
        "%b %d, %Y",
        "%d %B %Y",
        "%d %b %Y",
    ];

    for fmt in &datetime_fmts {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Ok(Value::Number(dt.and_utc().timestamp() as f64));
        }
    }
    for fmt in &date_fmts {
        if let Ok(d) = NaiveDate::parse_from_str(&s, fmt) {
            if let Some(dt) = d.and_hms_opt(0, 0, 0) {
                return Ok(Value::Number(dt.and_utc().timestamp() as f64));
            }
        }
    }

    Err(Signal::Error(format!(
        "date_parse: cannot parse '{}'. Pass a format string as the second argument, e.g. date_parse(s, \"%Y-%m-%d\")",
        s
    )))
}

/// date_format(timestamp_or_string, format?) → string
/// Default format: "%Y-%m-%d". Uses strftime directives.
pub fn date_format_impl(args: &[Value]) -> Result<Value, Signal> {
    let first = args.first().cloned().unwrap_or(Value::Null);
    let fmt = args
        .get(1)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "%Y-%m-%d".to_string());

    let ts = to_timestamp(&first, "date_format")?;
    let dt = Utc
        .timestamp_opt(ts, 0)
        .single()
        .ok_or_else(|| Signal::Error(format!("date_format: invalid timestamp {}", ts)))?;

    Ok(Value::Str(dt.format(&fmt).to_string()))
}

/// date_diff(date1, date2, unit?) → number
/// unit: "seconds" | "minutes" | "hours" | "days" | "weeks" | "months" | "years"
/// Positive when date2 > date1.
pub fn date_diff_impl(args: &[Value]) -> Result<Value, Signal> {
    let ts1 = to_timestamp(args.first().unwrap_or(&Value::Null), "date_diff")?;
    let ts2 = to_timestamp(args.get(1).unwrap_or(&Value::Null), "date_diff")?;
    let unit = args
        .get(2)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "seconds".to_string());

    let diff = ts2 - ts1;
    let result = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "sec" | "s" => diff as f64,
        "minutes" | "minute" | "min" => diff as f64 / 60.0,
        "hours" | "hour" | "h" => diff as f64 / 3_600.0,
        "days" | "day" | "d" => diff as f64 / 86_400.0,
        "weeks" | "week" | "w" => diff as f64 / 604_800.0,
        "months" | "month" | "mo" => diff as f64 / 2_592_000.0,
        "years" | "year" | "y" => diff as f64 / 31_536_000.0,
        other => {
            return Err(Signal::Error(format!(
                "date_diff: unknown unit '{}'. Use: seconds, minutes, hours, days, weeks, months, years",
                other
            )))
        }
    };
    Ok(Value::Number(result))
}

/// date_add(date, amount, unit) → Unix timestamp (number)
///
/// **This does not round-trip its own input's representation.** Every
/// other date-producing builtin in this module deals in ISO-8601 strings
/// (`date_now()`, and `date_parts(...).iso`) except this one and
/// `date_parse`/`date_from_parts`, which are numeric-timestamp-canonical
/// like the rest of the module's *internal* representation — it's only
/// `date_now()` that's the odd one out on the *output* side, even though
/// every function here accepts either shape on *input* (see
/// `to_timestamp`). Concretely: `date_add(date_now(), 4, "days")` takes a
/// string and returns a number — a real, reported bug (AgentX feedback,
/// 2026-07, item 1.2), because storing that number into a column that
/// otherwise holds ISO strings and later comparing it against
/// `date_now()` in SQL produces a **string comparison** between a
/// numeric-looking string and an ISO string, which is silently always
/// true (`"1784369596"` sorts before any `"2026-..."` string
/// lexicographically) regardless of the real date — a follow-up action
/// fires immediately instead of after N days, with no error anywhere.
///
/// Changing `date_add`'s return type is a breaking change for any
/// existing call site relying on the current numeric result (arithmetic
/// on it, storing it in a numeric column, comparing it to
/// `date_timestamp()`), so it stays as-is here. Use [`date_add_iso_impl`]
/// (the `date_add_iso` builtin) instead when you want the safe,
/// string-in-string-out behavior — or wrap this call:
/// `date_parts(date_add(...)).iso`.
pub fn date_add_impl(args: &[Value]) -> Result<Value, Signal> {
    let ts = to_timestamp(args.first().unwrap_or(&Value::Null), "date_add")?;
    let amount =
        args.get(1)
            .and_then(|v| v.as_number())
            .ok_or_else(|| Signal::Error("date_add(date, amount, unit)".into()))? as i64;
    let unit = args
        .get(2)
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_else(|| "days".to_string());

    let secs = match unit.to_lowercase().as_str() {
        "seconds" | "second" | "sec" | "s" => amount,
        "minutes" | "minute" | "min" => amount * 60,
        "hours" | "hour" | "h" => amount * 3_600,
        "days" | "day" | "d" => amount * 86_400,
        "weeks" | "week" | "w" => amount * 604_800,
        "months" | "month" | "mo" => amount * 2_592_000,
        "years" | "year" | "y" => amount * 31_536_000,
        other => {
            return Err(Signal::Error(format!(
            "date_add: unknown unit '{}'. Use: seconds, minutes, hours, days, weeks, months, years",
            other
        )))
        }
    };

    Ok(Value::Number((ts + secs) as f64))
}

/// date_add_iso(date, amount, unit) → ISO-8601 string
///
/// Same arithmetic as `date_add`, but always returns an ISO-8601 string —
/// matching `date_now()`'s representation, so the common
/// `date_add_iso(date_now(), 4, "days")` round-trips string-in-string-out
/// with no separate `date_parts(...).iso` unwrap needed. This is the
/// recommended function for computing a future/past date to store or
/// compare as a string (e.g. a `next_action_at` column also populated by
/// `date_now()`); see `date_add_impl`'s doc for why `date_add` itself
/// keeps its existing numeric return instead of changing to match.
pub fn date_add_iso_impl(args: &[Value]) -> Result<Value, Signal> {
    match date_add_impl(args)? {
        Value::Number(ts) => {
            let dt = Utc
                .timestamp_opt(ts as i64, 0)
                .single()
                .ok_or_else(|| Signal::Error(format!("date_add_iso: invalid timestamp {}", ts)))?;
            Ok(Value::Str(dt.to_rfc3339()))
        }
        other => Ok(other),
    }
}

/// date_parts(date) → { year, month, day, hour, minute, second, weekday, timestamp }
pub fn date_parts_impl(args: &[Value]) -> Result<Value, Signal> {
    let ts = to_timestamp(args.first().unwrap_or(&Value::Null), "date_parts")?;
    let dt = Utc
        .timestamp_opt(ts, 0)
        .single()
        .ok_or_else(|| Signal::Error("date_parts: invalid timestamp".into()))?;

    let mut map = HashMap::new();
    map.insert("year".into(), Value::Number(dt.year() as f64));
    map.insert("month".into(), Value::Number(dt.month() as f64));
    map.insert("day".into(), Value::Number(dt.day() as f64));
    map.insert("hour".into(), Value::Number(dt.hour() as f64));
    map.insert("minute".into(), Value::Number(dt.minute() as f64));
    map.insert("second".into(), Value::Number(dt.second() as f64));
    map.insert(
        "weekday".into(),
        Value::Number(dt.weekday().num_days_from_monday() as f64),
    );
    map.insert(
        "weekday_name".into(),
        Value::Str(dt.format("%A").to_string()),
    );
    map.insert("timestamp".into(), Value::Number(ts as f64));
    map.insert("iso".into(), Value::Str(dt.to_rfc3339()));
    Ok(Value::Object(map))
}

/// date_from_parts(year, month, day, hour?, minute?, second?) → Unix timestamp
pub fn date_from_parts_impl(args: &[Value]) -> Result<Value, Signal> {
    let year = args.first().and_then(|v| v.as_number()).ok_or_else(|| {
        Signal::Error("date_from_parts(year, month, day, hour?, minute?, second?)".into())
    })? as i32;
    let month = args.get(1).and_then(|v| v.as_number()).unwrap_or(1.0) as u32;
    let day = args.get(2).and_then(|v| v.as_number()).unwrap_or(1.0) as u32;
    let hour = args.get(3).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let minute = args.get(4).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;
    let second = args.get(5).and_then(|v| v.as_number()).unwrap_or(0.0) as u32;

    let dt = NaiveDate::from_ymd_opt(year, month, day)
        .and_then(|d| d.and_hms_opt(hour, minute, second))
        .ok_or_else(|| {
            Signal::Error(format!(
                "date_from_parts: invalid date {}-{:02}-{:02} {:02}:{:02}:{:02}",
                year, month, day, hour, minute, second
            ))
        })?;

    Ok(Value::Number(dt.and_utc().timestamp() as f64))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn to_timestamp(v: &Value, ctx: &str) -> Result<i64, Signal> {
    match v {
        Value::Number(n) => Ok(*n as i64),
        Value::Str(s) => match date_parse_impl(&[Value::Str(s.clone())]) {
            Ok(Value::Number(n)) => Ok(n as i64),
            _ => Err(Signal::Error(format!(
                "{}: cannot parse date string '{}'",
                ctx, s
            ))),
        },
        other => Err(Signal::Error(format!(
            "{}: expected timestamp (number) or date string, got {}",
            ctx,
            other.type_name()
        ))),
    }
}

fn parse_with_format(s: &str, fmt: &str) -> Result<Value, Signal> {
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
        return Ok(Value::Number(dt.and_utc().timestamp() as f64));
    }
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Ok(Value::Number(dt.and_utc().timestamp() as f64));
        }
    }
    Err(Signal::Error(format!(
        "date_parse: cannot parse '{}' with format '{}'",
        s, fmt
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Confirms the exact type asymmetry from the AgentX feedback (item
    // 1.2): date_now() -> string, but date_add(that string, ...) ->
    // number, even though date_add happily *accepts* a string as input.
    // This isn't being changed (see date_add_impl's doc for why), so this
    // test exists to pin the current, documented behavior rather than
    // let it drift silently.
    fn date_add_returns_a_number_even_when_given_a_string_input() {
        let now = date_now_impl().unwrap();
        assert!(matches!(now, Value::Str(_)));
        let future = date_add_impl(&[now, Value::Number(4.0), Value::Str("days".into())]).unwrap();
        assert!(
            matches!(future, Value::Number(_)),
            "date_add's return type is documented as staying numeric; got {:?}",
            future
        );
    }

    #[test]
    // date_add_iso is the recommended fix for the above: string in,
    // string out, matching date_now()'s own representation, so a
    // round-trip like `next_action_at = date_add_iso(date_now(), 4,
    // "days")` stays comparable against another date_now() value in a
    // string-typed store.
    fn date_add_iso_round_trips_a_string_input_to_a_string_output() {
        let now = date_now_impl().unwrap();
        let future =
            date_add_iso_impl(&[now, Value::Number(4.0), Value::Str("days".into())]).unwrap();
        let Value::Str(iso) = future else {
            panic!("expected date_add_iso to return a string, got {:?}", future);
        };
        // Must actually be a valid ISO-8601/RFC3339 string, not just any string.
        assert!(DateTime::parse_from_rfc3339(&iso).is_ok());
    }

    #[test]
    fn date_add_iso_is_four_days_after_date_now() {
        let now_ts = date_timestamp_impl().unwrap();
        let Value::Number(now_secs) = now_ts else {
            unreachable!()
        };
        let now_iso = date_now_impl().unwrap();
        let future =
            date_add_iso_impl(&[now_iso, Value::Number(4.0), Value::Str("days".into())]).unwrap();
        let Value::Str(iso) = future else {
            panic!("expected a string");
        };
        let future_ts = DateTime::parse_from_rfc3339(&iso).unwrap().timestamp() as f64;
        // Allow a couple of seconds of slack between date_timestamp_impl()
        // and date_now_impl()'s own `Utc::now()` calls.
        assert!((future_ts - (now_secs + 4.0 * 86_400.0)).abs() < 5.0);
    }
}
