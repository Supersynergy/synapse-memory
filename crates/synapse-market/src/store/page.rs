/// 64KB fixed page with Hilbert-zorder locality for (timestamp, price-zone).
/// Layout: 64B header | 6 column buffers (ts [i64], o/h/l/c/v [f32])
use blake3;

pub const PAGE_SIZE: usize = 65536;
pub const HEADER_SIZE: usize = 64;
/// Max rows per page: (64KB - 64B header) / (8B ts + 4B×5 ohlcv) = 65472/28 = 2338
pub const MAX_ROWS: usize = 2338;

/// 64-byte page header (packed manually, no padding issues)
#[derive(Debug, Clone, Copy)]
pub struct PageHeader {
    pub ts_min: i64,
    pub ts_max: i64,
    pub row_count: u32,
    pub checksum: [u8; 8],    // first 8 bytes of blake3
    pub hilbert_curve_id: u8, // 0 = unsorted, 1 = sorted by hilbert_index
    _pad: [u8; 35],
}

impl PageHeader {
    pub fn new(ts_min: i64, ts_max: i64, row_count: u32, body: &[u8]) -> Self {
        let hash = blake3::hash(body);
        let mut checksum = [0u8; 8];
        checksum.copy_from_slice(&hash.as_bytes()[..8]);
        Self {
            ts_min,
            ts_max,
            row_count,
            checksum,
            hilbert_curve_id: 0,
            _pad: [0u8; 35],
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.ts_min.to_le_bytes());
        buf[8..16].copy_from_slice(&self.ts_max.to_le_bytes());
        buf[16..20].copy_from_slice(&self.row_count.to_le_bytes());
        buf[20..28].copy_from_slice(&self.checksum);
        buf[28] = self.hilbert_curve_id;
        buf
    }

    pub fn from_bytes(b: &[u8; HEADER_SIZE]) -> Self {
        let ts_min = i64::from_le_bytes(b[0..8].try_into().unwrap());
        let ts_max = i64::from_le_bytes(b[8..16].try_into().unwrap());
        let row_count = u32::from_le_bytes(b[16..20].try_into().unwrap());
        let mut checksum = [0u8; 8];
        checksum.copy_from_slice(&b[20..28]);
        let hilbert_curve_id = b[28];
        Self {
            ts_min,
            ts_max,
            row_count,
            checksum,
            hilbert_curve_id,
            _pad: [0u8; 35],
        }
    }
}

/// A candle bar (f32 for storage efficiency per design-doc)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub ts: i64,
    pub open: f32,
    pub high: f32,
    pub low: f32,
    pub close: f32,
    pub volume: f32,
}

/// True Hilbert index via fast-hilbert crate for (timestamp, log-price) → u64 curve position.
pub fn hilbert_index(
    ts: i64,
    price: f32,
    ts_origin: i64,
    price_origin: f32,
    ts_scale: f64,
    price_scale: f64,
) -> u64 {
    let t = ((ts - ts_origin) as f64 / ts_scale).clamp(0.0, u32::MAX as f64) as u32;
    let p_log =
        ((price / price_origin).ln() as f64 / price_scale).clamp(0.0, u32::MAX as f64) as u32;
    fast_hilbert::xy2h::<u32>(t, p_log, 32)
}

/// Hilbert-curve order key for (ts_bucket, price_zone) — used to sort rows within a page
/// for spatial locality.  We keep it simple: interleave top 16 bits of ts_bucket and
/// price_zone into a 32-bit key.
pub fn hilbert_key(ts_bucket: u16, price_zone: u16) -> u32 {
    interleave_bits(ts_bucket as u32, price_zone as u32)
}

fn interleave_bits(mut x: u32, mut y: u32) -> u32 {
    // spread bits of x into even positions, y into odd
    x = (x | (x << 8)) & 0x00FF_00FF;
    x = (x | (x << 4)) & 0x0F0F_0F0F;
    x = (x | (x << 2)) & 0x3333_3333;
    x = (x | (x << 1)) & 0x5555_5555;
    y = (y | (y << 8)) & 0x00FF_00FF;
    y = (y | (y << 4)) & 0x0F0F_0F0F;
    y = (y | (y << 2)) & 0x3333_3333;
    y = (y | (y << 1)) & 0x5555_5555;
    x | (y << 1)
}

/// Encode a slice of bars into a page buffer (header + columnar body).
/// Bars are sorted by hilbert_index before encoding. Returns the filled buffer (PAGE_SIZE bytes).
pub fn encode_page(bars: &[Bar]) -> Vec<u8> {
    assert!(!bars.is_empty());
    assert!(bars.len() <= MAX_ROWS);

    // Sort by hilbert index for spatial locality
    let mut sorted: Vec<Bar> = bars.to_vec();
    let ts_origin = sorted.iter().map(|b| b.ts).min().unwrap();
    let price_origin = sorted.iter().map(|b| b.close).fold(f32::INFINITY, f32::min);
    let ts_range = (sorted.iter().map(|b| b.ts).max().unwrap() - ts_origin).max(1);
    let price_max = sorted
        .iter()
        .map(|b| b.close)
        .fold(f32::NEG_INFINITY, f32::max);
    let price_log_max = (price_max / price_origin).ln().max(1e-9_f64 as f32) as f64;
    let ts_scale = ts_range as f64 / u32::MAX as f64;
    let price_scale = price_log_max / u32::MAX as f64;
    let ts_scale = if ts_scale == 0.0 { 1.0 } else { ts_scale };
    let price_scale = if price_scale == 0.0 { 1.0 } else { price_scale };
    sorted.sort_unstable_by_key(|b| {
        hilbert_index(
            b.ts,
            b.close,
            ts_origin,
            price_origin,
            ts_scale,
            price_scale,
        )
    });
    let bars = sorted.as_slice();

    let n = bars.len();

    // body: ts (i64 × n) | open (f32 × n) | high | low | close | volume
    let body_len = n * (8 + 4 + 4 + 4 + 4 + 4); // 28 bytes/row
    let mut body = vec![0u8; body_len];

    let o_off = n * 8;
    let h_off = n * 8 + n * 4;
    let l_off = n * 8 + n * 8;
    let c_off = n * 8 + n * 12;
    let v_off = n * 8 + n * 16;

    for (i, bar) in bars.iter().enumerate() {
        body[i * 8..i * 8 + 8].copy_from_slice(&bar.ts.to_le_bytes());
        body[o_off + i * 4..o_off + i * 4 + 4].copy_from_slice(&bar.open.to_le_bytes());
        body[h_off + i * 4..h_off + i * 4 + 4].copy_from_slice(&bar.high.to_le_bytes());
        body[l_off + i * 4..l_off + i * 4 + 4].copy_from_slice(&bar.low.to_le_bytes());
        body[c_off + i * 4..c_off + i * 4 + 4].copy_from_slice(&bar.close.to_le_bytes());
        body[v_off + i * 4..v_off + i * 4 + 4].copy_from_slice(&bar.volume.to_le_bytes());
    }

    let ts_min = bars.iter().map(|b| b.ts).min().unwrap();
    let ts_max = bars.iter().map(|b| b.ts).max().unwrap();
    let mut header = PageHeader::new(ts_min, ts_max, n as u32, &body);
    header.hilbert_curve_id = 1;

    let mut page = vec![0u8; PAGE_SIZE];
    page[..HEADER_SIZE].copy_from_slice(&header.to_bytes());
    let copy_len = body_len.min(PAGE_SIZE - HEADER_SIZE);
    page[HEADER_SIZE..HEADER_SIZE + copy_len].copy_from_slice(&body[..copy_len]);
    page
}

/// SoA in-memory page — parallel column Vecs, no per-row structs.
/// Build from a `&[Bar]` slice; public Bar struct unchanged.
pub struct Page {
    ts: Vec<i64>,
    open: Vec<f32>,
    high: Vec<f32>,
    low: Vec<f32>,
    close: Vec<f32>,
    volume: Vec<f32>,
}

impl Page {
    pub fn from_bars(bars: &[Bar]) -> Self {
        let mut sorted: Vec<Bar> = bars.to_vec();
        if !sorted.is_empty() {
            let ts_origin = sorted.iter().map(|b| b.ts).min().unwrap();
            let price_origin = sorted.iter().map(|b| b.close).fold(f32::INFINITY, f32::min);
            let price_origin = if price_origin <= 0.0 {
                1.0_f32
            } else {
                price_origin
            };
            let ts_range = (sorted.iter().map(|b| b.ts).max().unwrap() - ts_origin).max(1);
            let price_max = sorted
                .iter()
                .map(|b| b.close)
                .fold(f32::NEG_INFINITY, f32::max);
            let price_log_max = (price_max / price_origin).ln().max(1e-9) as f64;
            let ts_scale = (ts_range as f64 / u32::MAX as f64).max(1.0);
            let price_scale = price_log_max.max(1e-9) / u32::MAX as f64;
            sorted.sort_unstable_by_key(|b| {
                hilbert_index(
                    b.ts,
                    b.close,
                    ts_origin,
                    price_origin,
                    ts_scale,
                    price_scale,
                )
            });
        }
        let n = sorted.len();
        let mut ts = Vec::with_capacity(n);
        let mut open = Vec::with_capacity(n);
        let mut high = Vec::with_capacity(n);
        let mut low = Vec::with_capacity(n);
        let mut close = Vec::with_capacity(n);
        let mut volume = Vec::with_capacity(n);
        for b in &sorted {
            ts.push(b.ts);
            open.push(b.open);
            high.push(b.high);
            low.push(b.low);
            close.push(b.close);
            volume.push(b.volume);
        }
        Self {
            ts,
            open,
            high,
            low,
            close,
            volume,
        }
    }

    #[inline]
    pub fn closes(&self) -> &[f32] {
        &self.close
    }
    #[inline]
    pub fn opens(&self) -> &[f32] {
        &self.open
    }
    #[inline]
    pub fn highs(&self) -> &[f32] {
        &self.high
    }
    #[inline]
    pub fn lows(&self) -> &[f32] {
        &self.low
    }
    #[inline]
    pub fn volumes(&self) -> &[f32] {
        &self.volume
    }
    #[inline]
    pub fn timestamps(&self) -> &[i64] {
        &self.ts
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.ts.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.ts.is_empty()
    }
}

/// Column selector for range_columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Col {
    Ts,
    Open,
    High,
    Low,
    Close,
    Volume,
}

/// Returned by range_columns — parallel column vecs, same length.
pub struct ColumnSlices {
    pub ts: Vec<i64>,
    pub open: Option<Vec<f32>>,
    pub high: Option<Vec<f32>>,
    pub low: Option<Vec<f32>>,
    pub close: Option<Vec<f32>>,
    pub volume: Option<Vec<f32>>,
}

/// Decode page directly into columnar Page — zero intermediate Vec<Bar>.
/// Filters by ts range in one pass.
pub fn decode_page_soa_filtered(page: &[u8], ts_start: i64, ts_end: i64) -> Page {
    let hdr_bytes: [u8; HEADER_SIZE] = page[..HEADER_SIZE].try_into().unwrap();
    let header = PageHeader::from_bytes(&hdr_bytes);
    let n = header.row_count as usize;
    let body = &page[HEADER_SIZE..];
    let o_off = n * 8;
    let h_off = n * 8 + n * 4;
    let l_off = n * 8 + n * 8;
    let c_off = n * 8 + n * 12;
    let v_off = n * 8 + n * 16;

    let mut ts_vec = Vec::with_capacity(n);
    let mut open_vec = Vec::with_capacity(n);
    let mut high_vec = Vec::with_capacity(n);
    let mut low_vec = Vec::with_capacity(n);
    let mut close_vec = Vec::with_capacity(n);
    let mut vol_vec = Vec::with_capacity(n);

    for i in 0..n {
        let ts = i64::from_le_bytes(body[i * 8..i * 8 + 8].try_into().unwrap());
        if ts < ts_start || ts >= ts_end {
            continue;
        }
        ts_vec.push(ts);
        open_vec.push(f32::from_le_bytes(
            body[o_off + i * 4..o_off + i * 4 + 4].try_into().unwrap(),
        ));
        high_vec.push(f32::from_le_bytes(
            body[h_off + i * 4..h_off + i * 4 + 4].try_into().unwrap(),
        ));
        low_vec.push(f32::from_le_bytes(
            body[l_off + i * 4..l_off + i * 4 + 4].try_into().unwrap(),
        ));
        close_vec.push(f32::from_le_bytes(
            body[c_off + i * 4..c_off + i * 4 + 4].try_into().unwrap(),
        ));
        vol_vec.push(f32::from_le_bytes(
            body[v_off + i * 4..v_off + i * 4 + 4].try_into().unwrap(),
        ));
    }
    Page {
        ts: ts_vec,
        open: open_vec,
        high: high_vec,
        low: low_vec,
        close: close_vec,
        volume: vol_vec,
    }
}

/// Decode all bars from a page buffer.
pub fn decode_page(page: &[u8]) -> (PageHeader, Vec<Bar>) {
    let hdr_bytes: [u8; HEADER_SIZE] = page[..HEADER_SIZE].try_into().unwrap();
    let header = PageHeader::from_bytes(&hdr_bytes);
    let n = header.row_count as usize;

    let body = &page[HEADER_SIZE..];
    let o_off = n * 8;
    let h_off = n * 8 + n * 4;
    let l_off = n * 8 + n * 8;
    let c_off = n * 8 + n * 12;
    let v_off = n * 8 + n * 16;

    let mut bars = Vec::with_capacity(n);
    for i in 0..n {
        let ts = i64::from_le_bytes(body[i * 8..i * 8 + 8].try_into().unwrap());
        let open = f32::from_le_bytes(body[o_off + i * 4..o_off + i * 4 + 4].try_into().unwrap());
        let high = f32::from_le_bytes(body[h_off + i * 4..h_off + i * 4 + 4].try_into().unwrap());
        let low = f32::from_le_bytes(body[l_off + i * 4..l_off + i * 4 + 4].try_into().unwrap());
        let close = f32::from_le_bytes(body[c_off + i * 4..c_off + i * 4 + 4].try_into().unwrap());
        let volume = f32::from_le_bytes(body[v_off + i * 4..v_off + i * 4 + 4].try_into().unwrap());
        bars.push(Bar {
            ts,
            open,
            high,
            low,
            close,
            volume,
        });
    }
    (header, bars)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bars(n: usize) -> Vec<Bar> {
        (0..n)
            .map(|i| Bar {
                ts: 1_700_000_000 + i as i64 * 900,
                open: 100.0 + i as f32 * 0.1,
                high: 101.0 + i as f32 * 0.1,
                low: 99.0 + i as f32 * 0.1,
                close: 100.5 + i as f32 * 0.1,
                volume: 1000.0 + i as f32,
            })
            .collect()
    }

    #[test]
    fn roundtrip_page() {
        let bars = make_bars(100);
        let page = encode_page(&bars);
        assert_eq!(page.len(), PAGE_SIZE);
        let (hdr, decoded) = decode_page(&page);
        assert_eq!(hdr.row_count, 100);
        assert_eq!(decoded.len(), 100);
        // Hilbert-sort reorders rows; compare as ts-sorted sets
        let mut inp = bars.clone();
        let mut dec = decoded.clone();
        inp.sort_by_key(|b| b.ts);
        dec.sort_by_key(|b| b.ts);
        for (a, b) in inp.iter().zip(dec.iter()) {
            assert_eq!(a.ts, b.ts);
            assert!((a.close - b.close).abs() < 1e-5);
        }
    }

    #[test]
    fn hilbert_key_monotone_ts() {
        // Same price-zone, ascending ts → keys should increase
        let k1 = hilbert_key(0, 5);
        let k2 = hilbert_key(1, 5);
        assert!(k2 > k1);
    }
}
