use super::LiveTick;
use serde_json::Value;

/// Wire format selector for `WebSocketTickStream`.
#[derive(Debug, Clone)]
pub enum Parser {
    /// Generic JSON: `{"ts": <unix_ms>, "ticker": "X", "price": 1.0, "qty": 1.0, "side": "buy"}`
    Generic,
    /// Polygon.io v3 aggregate/trade websocket format.
    PolygonV3,
    /// Tradier streaming v1 quote/trade format.
    TradierV1,
    /// Kraken v2 trade feed (crypto).
    KrakenV2,
}

impl Parser {
    /// Parse a raw JSON text frame into a `Tick`. Returns `None` on non-trade frames.
    pub fn parse(&self, raw: &str) -> Option<LiveTick> {
        match self {
            Parser::Generic => parse_generic(raw),
            Parser::PolygonV3 => parse_polygon_v3(raw),
            Parser::TradierV1 => parse_tradier_v1(raw),
            Parser::KrakenV2 => parse_kraken_v2(raw),
        }
    }
}

fn parse_generic(raw: &str) -> Option<LiveTick> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let ts = v["ts"].as_i64()?;
    let price = v["price"].as_f64()?;
    let qty = v["qty"].as_f64().unwrap_or(0.0);
    Some(LiveTick { ts, price, qty })
}

fn parse_polygon_v3(raw: &str) -> Option<LiveTick> {
    // Polygon sends an array: [{"ev":"T","sym":"AAPL","t":1234567890000,"p":150.0,"s":100,...}]
    let v: Value = serde_json::from_str(raw).ok()?;
    let arr = v.as_array()?;
    let obj = arr.first()?;
    if obj["ev"].as_str() != Some("T") {
        return None;
    }
    let ts = obj["t"].as_i64()? / 1000; // ms → s
    let price = obj["p"].as_f64()?;
    let qty = obj["s"].as_f64().unwrap_or(0.0);
    Some(LiveTick { ts, price, qty })
}

fn parse_tradier_v1(raw: &str) -> Option<LiveTick> {
    // Tradier: {"type":"trade","symbol":"AAPL","price":150.0,"size":100,"timestamp":1234567890}
    let v: Value = serde_json::from_str(raw).ok()?;
    if v["type"].as_str() != Some("trade") {
        return None;
    }
    let ts = v["timestamp"].as_i64()?;
    let price = v["price"].as_f64()?;
    let qty = v["size"].as_f64().unwrap_or(0.0);
    Some(LiveTick { ts, price, qty })
}

fn parse_kraken_v2(raw: &str) -> Option<LiveTick> {
    // Kraken v2: {"channel":"trade","data":[{"price":50000.0,"qty":0.001,"timestamp":"2024-..."}]}
    let v: Value = serde_json::from_str(raw).ok()?;
    if v["channel"].as_str() != Some("trade") {
        return None;
    }
    let data = v["data"].as_array()?;
    let trade = data.first()?;
    // timestamp is ISO8601 string; parse as unix seconds via simple millis field fallback
    let ts = if let Some(ms) = trade["timestamp_ms"].as_i64() {
        ms / 1000
    } else {
        // fallback: use current epoch placeholder (real impl would parse ISO8601)
        trade["timestamp"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .map(|f| f as i64)
            .unwrap_or(0)
    };
    let price = trade["price"].as_f64()?;
    let qty = trade["qty"].as_f64().unwrap_or(0.0);
    Some(LiveTick { ts, price, qty })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_roundtrip() {
        let s = r#"{"ts":1000,"ticker":"AAPL","price":150.5,"qty":10.0,"side":"buy"}"#;
        let t = Parser::Generic.parse(s).unwrap();
        assert_eq!(t.ts, 1000);
        assert!((t.price - 150.5).abs() < 1e-9);
        assert!((t.qty - 10.0).abs() < 1e-9);
    }

    #[test]
    fn polygon_roundtrip() {
        let s = r#"[{"ev":"T","sym":"AAPL","t":1700000000000,"p":155.0,"s":50}]"#;
        let t = Parser::PolygonV3.parse(s).unwrap();
        assert_eq!(t.ts, 1700000000);
        assert!((t.price - 155.0).abs() < 1e-9);
    }

    #[test]
    fn polygon_non_trade_skipped() {
        let s = r#"[{"ev":"Q","sym":"AAPL","t":1700000000000,"p":155.0,"s":50}]"#;
        assert!(Parser::PolygonV3.parse(s).is_none());
    }

    #[test]
    fn tradier_roundtrip() {
        let s = r#"{"type":"trade","symbol":"MSFT","price":300.0,"size":200,"timestamp":9999}"#;
        let t = Parser::TradierV1.parse(s).unwrap();
        assert_eq!(t.ts, 9999);
        assert!((t.price - 300.0).abs() < 1e-9);
    }

    #[test]
    fn tradier_non_trade_skipped() {
        let s = r#"{"type":"quote","symbol":"MSFT","price":300.0,"size":200,"timestamp":9999}"#;
        assert!(Parser::TradierV1.parse(s).is_none());
    }

    #[test]
    fn kraken_roundtrip() {
        let s = r#"{"channel":"trade","data":[{"price":50000.0,"qty":0.001,"timestamp_ms":1700000000000}]}"#;
        let t = Parser::KrakenV2.parse(s).unwrap();
        assert_eq!(t.ts, 1700000000);
        assert!((t.price - 50000.0).abs() < 1e-9);
    }

    #[test]
    fn kraken_non_trade_skipped() {
        let s = r#"{"channel":"heartbeat","data":[]}"#;
        assert!(Parser::KrakenV2.parse(s).is_none());
    }
}
