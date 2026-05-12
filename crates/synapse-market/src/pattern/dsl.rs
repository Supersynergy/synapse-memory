/// Hand-rolled DSL parser (~150 LoC).
///
/// Grammar:
/// ```text
/// expr   = term (("AND" | "OR") term)*
///        | term "THEN" term "within" number
/// term   = atom | "(" expr ")"
/// atom   = "DroughtBuy" "(" params ")"
///        | "InsiderCluster" "(" params ")"
///        | "FdaTriple" "(" params ")"
///        | "VolumeSpike" "(" params ")"
/// params = key "=" value ("," key "=" value)*
/// ```
use super::Pattern;
use crate::error::{Error, Result};
use std::collections::HashMap;

pub fn parse(input: &str) -> Result<Pattern> {
    let tokens = tokenize(input);
    let mut pos = 0usize;
    let p = parse_expr(&tokens, &mut pos)?;
    if pos < tokens.len() {
        return Err(Error::Other(format!("unexpected token: {}", tokens[pos])));
    }
    Ok(p)
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '(' | ')' | ',' | '=' => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
                cur = String::new();
                out.push(c.to_string());
            }
            ' ' | '\t' | '\n' | '\r' => {
                if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
                cur = String::new();
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() { out.push(cur.trim().to_string()); }
    out
}

fn parse_expr(toks: &[String], pos: &mut usize) -> Result<Pattern> {
    let left = parse_then(toks, pos)?;

    // AND / OR
    loop {
        match toks.get(*pos).map(|s| s.as_str()) {
            Some("AND") => {
                *pos += 1;
                let right = parse_then(toks, pos)?;
                return Ok(Pattern::And(Box::new(left), Box::new(right)));
            }
            Some("OR") => {
                *pos += 1;
                let right = parse_then(toks, pos)?;
                return Ok(Pattern::Or(Box::new(left), Box::new(right)));
            }
            _ => break,
        }
    }
    Ok(left)
}

fn parse_then(toks: &[String], pos: &mut usize) -> Result<Pattern> {
    let a = parse_term(toks, pos)?;
    if toks.get(*pos).map(|s| s.as_str()) == Some("THEN") {
        *pos += 1;
        let b = parse_term(toks, pos)?;
        // optional "within N"
        let within = if toks.get(*pos).map(|s| s.as_str()) == Some("within") {
            *pos += 1;
            let n = parse_u32(toks, pos)?;
            n
        } else {
            30
        };
        return Ok(Pattern::Then(Box::new(a), Box::new(b), within));
    }
    Ok(a)
}

fn parse_term(toks: &[String], pos: &mut usize) -> Result<Pattern> {
    if toks.get(*pos).map(|s| s.as_str()) == Some("(") {
        *pos += 1;
        let p = parse_expr(toks, pos)?;
        expect(toks, pos, ")")?;
        return Ok(p);
    }
    parse_atom(toks, pos)
}

fn parse_atom(toks: &[String], pos: &mut usize) -> Result<Pattern> {
    let name = toks.get(*pos).ok_or_else(|| Error::Other("expected pattern name".into()))?.clone();
    *pos += 1;
    expect(toks, pos, "(")?;
    let params = parse_params(toks, pos)?;
    expect(toks, pos, ")")?;

    match name.as_str() {
        "DroughtBuy" => {
            let quiet = get_u32(&params, "quiet")?;
            let min = get_f64(&params, "min")?;
            Ok(Pattern::DroughtBuy { quiet_days: quiet, min_value: min })
        }
        "InsiderCluster" => {
            let k = get_u32(&params, "k")?;
            let w = get_u32(&params, "w")?;
            Ok(Pattern::InsiderCluster { k, window_days: w })
        }
        "FdaTriple" => {
            let w = get_u32(&params, "w")?;
            Ok(Pattern::FdaTriple { window_days: w })
        }
        "VolumeSpike" => {
            let mult = get_f32(&params, "mult")?;
            let w = get_u32(&params, "w")?;
            Ok(Pattern::VolumeSpike { multiplier: mult, window_bars: w })
        }
        other => Err(Error::Other(format!("unknown pattern: {other}"))),
    }
}

fn parse_params(toks: &[String], pos: &mut usize) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    while toks.get(*pos).map(|s| s.as_str()) != Some(")") && *pos < toks.len() {
        let key = toks.get(*pos).ok_or_else(|| Error::Other("expected key".into()))?.clone();
        *pos += 1;
        expect(toks, pos, "=")?;
        let val = toks.get(*pos).ok_or_else(|| Error::Other("expected value".into()))?.clone();
        *pos += 1;
        map.insert(key, val);
        if toks.get(*pos).map(|s| s.as_str()) == Some(",") { *pos += 1; }
    }
    Ok(map)
}

fn expect(toks: &[String], pos: &mut usize, tok: &str) -> Result<()> {
    match toks.get(*pos) {
        Some(t) if t == tok => { *pos += 1; Ok(()) }
        other => Err(Error::Other(format!("expected `{tok}`, got `{:?}`", other))),
    }
}

fn parse_u32(toks: &[String], pos: &mut usize) -> Result<u32> {
    let s = toks.get(*pos).ok_or_else(|| Error::Other("expected number".into()))?;
    let n: u32 = s.parse().map_err(|_| Error::Other(format!("not a u32: {s}")))?;
    *pos += 1;
    Ok(n)
}

fn get_u32(map: &HashMap<String, String>, key: &str) -> Result<u32> {
    map.get(key).ok_or_else(|| Error::Other(format!("missing param: {key}")))?
        .parse().map_err(|_| Error::Other(format!("param {key} not u32")))
}

fn get_f64(map: &HashMap<String, String>, key: &str) -> Result<f64> {
    map.get(key).ok_or_else(|| Error::Other(format!("missing param: {key}")))?
        .parse().map_err(|_| Error::Other(format!("param {key} not f64")))
}

fn get_f32(map: &HashMap<String, String>, key: &str) -> Result<f32> {
    map.get(key).ok_or_else(|| Error::Other(format!("missing param: {key}")))?
        .parse().map_err(|_| Error::Other(format!("param {key} not f32")))
}
