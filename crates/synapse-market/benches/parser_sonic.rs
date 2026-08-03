// Before/after: serde_json vs sonic-rs for the live tick stream parser.
// Mirrors the field-extraction pattern in src/stream/parser.rs.
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use sonic_rs::{JsonContainerTrait, JsonValueTrait};

const GENERIC: &str = r#"{"ts":1700000000,"ticker":"AAPL","price":150.5,"qty":10.0,"side":"buy"}"#;
const POLYGON: &str = r#"[{"ev":"T","sym":"AAPL","t":1700000000000,"p":155.0,"s":50}]"#;
const TRADIER: &str =
    r#"{"type":"trade","symbol":"MSFT","price":300.0,"size":200,"timestamp":1700000000}"#;
const KRAKEN: &str =
    r#"{"channel":"trade","data":[{"price":50000.0,"qty":0.001,"timestamp_ms":1700000000000}]}"#;

// ---- serde_json path (current) ----
fn serde_generic(raw: &str) -> Option<(i64, f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some((
        v["ts"].as_i64()?,
        v["price"].as_f64()?,
        v["qty"].as_f64().unwrap_or(0.0),
    ))
}
fn serde_polygon(raw: &str) -> Option<(i64, f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let obj = v.as_array()?.first()?;
    if obj["ev"].as_str() != Some("T") {
        return None;
    }
    Some((
        obj["t"].as_i64()? / 1000,
        obj["p"].as_f64()?,
        obj["s"].as_f64().unwrap_or(0.0),
    ))
}
fn serde_kraken(raw: &str) -> Option<(i64, f64, f64)> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    if v["channel"].as_str() != Some("trade") {
        return None;
    }
    let t = v["data"].as_array()?.first()?;
    Some((
        t["timestamp_ms"].as_i64()? / 1000,
        t["price"].as_f64()?,
        t["qty"].as_f64().unwrap_or(0.0),
    ))
}

// ---- sonic-rs path (proposed) ----
fn sonic_generic(raw: &str) -> Option<(i64, f64, f64)> {
    let v: sonic_rs::Value = sonic_rs::from_str(raw).ok()?;
    Some((
        v["ts"].as_i64()?,
        v["price"].as_f64()?,
        v["qty"].as_f64().unwrap_or(0.0),
    ))
}
fn sonic_polygon(raw: &str) -> Option<(i64, f64, f64)> {
    let v: sonic_rs::Value = sonic_rs::from_str(raw).ok()?;
    let obj = v.as_array()?.iter().next()?;
    if obj["ev"].as_str() != Some("T") {
        return None;
    }
    Some((
        obj["t"].as_i64()? / 1000,
        obj["p"].as_f64()?,
        obj["s"].as_f64().unwrap_or(0.0),
    ))
}
fn sonic_kraken(raw: &str) -> Option<(i64, f64, f64)> {
    let v: sonic_rs::Value = sonic_rs::from_str(raw).ok()?;
    if v["channel"].as_str() != Some("trade") {
        return None;
    }
    let t = v["data"].as_array()?.iter().next()?;
    Some((
        t["timestamp_ms"].as_i64()? / 1000,
        t["price"].as_f64()?,
        t["qty"].as_f64().unwrap_or(0.0),
    ))
}

fn bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("tick_parse");
    g.bench_function("serde/generic", |b| {
        b.iter(|| serde_generic(black_box(GENERIC)))
    });
    g.bench_function("sonic/generic", |b| {
        b.iter(|| sonic_generic(black_box(GENERIC)))
    });
    g.bench_function("serde/polygon", |b| {
        b.iter(|| serde_polygon(black_box(POLYGON)))
    });
    g.bench_function("sonic/polygon", |b| {
        b.iter(|| sonic_polygon(black_box(POLYGON)))
    });
    g.bench_function("serde/tradier", |b| {
        b.iter(|| serde_generic(black_box(TRADIER)))
    });
    g.bench_function("sonic/tradier", |b| {
        b.iter(|| sonic_generic(black_box(TRADIER)))
    });
    g.bench_function("serde/kraken", |b| {
        b.iter(|| serde_kraken(black_box(KRAKEN)))
    });
    g.bench_function("sonic/kraken", |b| {
        b.iter(|| sonic_kraken(black_box(KRAKEN)))
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
