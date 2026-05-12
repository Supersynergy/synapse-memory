use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Side {
    Bid = 0,
    Ask = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Op {
    Insert = 0,
    Update = 1,
    Delete = 2,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BookEvent {
    pub ts: i64,
    pub level: u8,
    pub side: Side,
    pub op: Op,
    pub px: f32,
    pub qty: f32,
}

/// On-disk compact delta: 12 bytes per event.
/// ts_delta: i32 nanos from checkpoint ts
/// side_op: bits [7..4] = side, bits [3..0] = op
/// px_delta: i16 ticks (px * 100.0 as i16 delta from checkpoint base_px)
/// qty: i32 * 1000 fixed-point (max ~2.1M qty)
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub(crate) struct PackedEvent {
    pub ts_delta: i32, // 4
    pub level: u8,     // 1
    pub side_op: u8,   // 1
    pub px_delta: i16, // 2
    pub qty_raw: i32,  // 4
} // = 12 bytes

static_assertions::const_assert_eq!(std::mem::size_of::<PackedEvent>(), 12);

impl PackedEvent {
    pub fn encode(ev: &BookEvent, base_ts: i64, base_px: f32) -> Self {
        let ts_delta = (ev.ts - base_ts).min(i32::MAX as i64).max(i32::MIN as i64) as i32;
        let px_delta = ((ev.px - base_px) * 100.0)
            .round()
            .clamp(i16::MIN as f32, i16::MAX as f32) as i16;
        let qty_raw = (ev.qty * 1000.0)
            .round()
            .clamp(i32::MIN as f32, i32::MAX as f32) as i32;
        let side_op = ((ev.side as u8) << 4) | (ev.op as u8 & 0x0F);
        Self {
            ts_delta,
            level: ev.level,
            side_op,
            px_delta,
            qty_raw,
        }
    }

    pub fn decode(&self, base_ts: i64, base_px: f32) -> BookEvent {
        let ts = base_ts + self.ts_delta as i64;
        let side = if (self.side_op >> 4) == 0 {
            Side::Bid
        } else {
            Side::Ask
        };
        let op = match self.side_op & 0x0F {
            0 => Op::Insert,
            1 => Op::Update,
            _ => Op::Delete,
        };
        let px = base_px + (self.px_delta as f32) / 100.0;
        let qty = self.qty_raw as f32 / 1000.0;
        BookEvent {
            ts,
            level: self.level,
            side,
            op,
            px,
            qty,
        }
    }
}
