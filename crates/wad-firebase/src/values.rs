//! Firestore's typed values (REST form) to and from JSON. A timestamp has no
//! JSON form: `{"$time": <epoch ms>}` stands for one going in, and one coming
//! out is its RFC 3339 text (as Firestore gives it); `to_ms` reads either.

use serde_json::{Map, Value, json};

/// A timestamp field's JSON stand-in.
pub fn time(ms: i64) -> Value {
    json!({ "$time": ms })
}

pub fn to_value(v: &Value) -> Value {
    match v {
        Value::Null => json!({ "nullValue": null }),
        Value::Bool(b) => json!({ "booleanValue": b }),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({ "integerValue": n.to_string() }),
        Value::Number(n) => json!({ "doubleValue": n.as_f64() }),
        Value::String(s) => json!({ "stringValue": s }),
        Value::Array(a) => json!({ "arrayValue": { "values": a.iter().map(to_value).collect::<Vec<_>>() } }),
        Value::Object(o) => match (o.len(), o.get("$time").and_then(Value::as_i64)) {
            (1, Some(ms)) => json!({ "timestampValue": rfc3339(ms) }),
            _ => json!({ "mapValue": { "fields": to_fields(o) } }),
        },
    }
}

pub fn to_fields(o: &Map<String, Value>) -> Value {
    Value::Object(o.iter().map(|(k, v)| (k.clone(), to_value(v))).collect())
}

pub fn from_value(v: &Value) -> Value {
    let Some((kind, x)) = v.as_object().and_then(|o| o.iter().next()) else { return Value::Null };
    match kind.as_str() {
        "nullValue" => Value::Null,
        "integerValue" => x.as_str().and_then(|s| s.parse::<i64>().ok()).map(Value::from).unwrap_or(Value::Null),
        "mapValue" => from_fields(x.get("fields").unwrap_or(&Value::Null)),
        "arrayValue" => {
            Value::Array(x.get("values").and_then(Value::as_array).into_iter().flatten().map(from_value).collect())
        }
        // booleanValue, doubleValue, stringValue, timestampValue, referenceValue, ...
        _ => x.clone(),
    }
}

pub fn from_fields(fields: &Value) -> Value {
    Value::Object(fields.as_object().into_iter().flatten().map(|(k, v)| (k.clone(), from_value(v))).collect())
}

/// A document's id: the last segment of its name.
pub fn doc_id(doc: &Value) -> String {
    doc.get("name").and_then(Value::as_str).and_then(|n| n.rsplit('/').next()).unwrap_or_default().to_string()
}

/// A document's path under documents/ (`users/u/machines/m/commands/c`).
pub fn doc_path(doc: &Value) -> String {
    let name = doc.get("name").and_then(Value::as_str).unwrap_or_default();
    name.split_once("/documents/").map(|(_, p)| p.to_string()).unwrap_or_default()
}

// Days from 1970-01-01 to a civil date and back (Howard Hinnant's algorithms).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Epoch milliseconds as RFC 3339 UTC: 2026-10-02T09:41:06.129Z.
pub fn rfc3339(ms: i64) -> String {
    let (days, rest) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s, milli) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// A timestamp as epoch milliseconds: a number (Date.now()) as it is, or
/// RFC 3339 text (a Firestore Timestamp; up to nanoseconds, any offset).
/// 0 when it's neither.
pub fn to_ms(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_f64().map(|f| f as i64).unwrap_or(0),
        Value::String(s) => parse_rfc3339(s).unwrap_or(0),
        _ => 0,
    }
}

fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20
        || b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't' | b' ')
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let mut rest = &s[19..];
    let mut milli = 0;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
        let padded = format!("{digits:0<3}");
        milli = padded[..3].parse::<i64>().ok()?;
        rest = &frac[digits.len()..];
    }
    let offset_min = match rest {
        "Z" | "z" => 0,
        o if o.len() == 6 && (o.starts_with('+') || o.starts_with('-')) => {
            let v = o[1..3].parse::<i64>().ok()? * 60 + o[4..6].parse::<i64>().ok()?;
            if o.starts_with('-') { -v } else { v }
        }
        _ => return None,
    };
    let days = days_from_civil(y, mo, d);
    Some(((days * 86_400 + h * 3600 + mi * 60 + sec - offset_min * 60) * 1000) + milli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_both_ways() {
        let v = json!({"name": "Surface", "n": 3, "x": 1.5, "ok": true, "none": null, "list": ["a", 2], "map": {"k": "v"}, "seen": time(1_790_931_762_197)});
        let fields = to_fields(v.as_object().unwrap());
        assert_eq!(fields["n"], json!({"integerValue": "3"}));
        assert_eq!(fields["seen"], json!({"timestampValue": "2026-10-02T09:02:42.197Z"}));
        assert_eq!(fields["map"], json!({"mapValue": {"fields": {"k": {"stringValue": "v"}}}}));
        let back = from_fields(&fields);
        assert_eq!(back["list"], json!(["a", 2]));
        assert_eq!(back["seen"], "2026-10-02T09:02:42.197Z");
        assert_eq!(to_ms(&back["seen"]), 1_790_931_762_197);
        assert_eq!(from_value(&json!({"arrayValue": {}})), json!([]));
        assert_eq!(from_fields(&Value::Null), json!({}));
    }

    #[test]
    fn times() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(951_782_400_000), "2000-02-29T00:00:00.000Z");
        assert_eq!(to_ms(&json!("2000-02-29T00:00:00Z")), 951_782_400_000);
        assert_eq!(to_ms(&json!("2026-10-02T21:02:43.123456789Z")), to_ms(&json!("2026-10-02T21:02:43.123Z")));
        assert_eq!(to_ms(&json!("2026-10-02T21:02:43+12:00")), to_ms(&json!("2026-10-02T09:02:43Z")));
        assert_eq!(to_ms(&json!(1_700_000_000_000_i64)), 1_700_000_000_000);
        assert_eq!((to_ms(&json!("nope")), to_ms(&Value::Null)), (0, 0));
        for ms in [0, 1, 999, 86_399_999, 1_790_931_762_197, -1000] {
            assert_eq!(to_ms(&json!(rfc3339(ms))), ms);
        }
    }

    #[test]
    fn names() {
        let d = json!({"name": "projects/p/databases/(default)/documents/users/u/machines/m/commands/c1"});
        assert_eq!((doc_id(&d), doc_path(&d)), ("c1".to_string(), "users/u/machines/m/commands/c1".to_string()));
    }
}
