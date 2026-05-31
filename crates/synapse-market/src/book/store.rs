use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::event::{BookEvent, Op, PackedEvent, Side};

const MAGIC: u64 = 0x534D42_424F4F4Bu64; // "SMBBOOKK"
const VERSION: u32 = 1;
const EVENTS_PER_CHECKPOINT: usize = 1000;
const LEVELS: usize = 50;
const HEADER_SIZE: usize = 64;

/// Full 50-level snapshot stored at each checkpoint.
#[derive(Clone)]
pub struct BookSnapshot {
    pub bids: [(f32, f32); LEVELS],
    pub asks: [(f32, f32); LEVELS],
}

impl Default for BookSnapshot {
    fn default() -> Self {
        Self {
            bids: [(0.0, 0.0); LEVELS],
            asks: [(0.0, 0.0); LEVELS],
        }
    }
}

impl BookSnapshot {
    fn apply(&mut self, ev: &BookEvent) {
        let slot = &mut match ev.side {
            Side::Bid => &mut self.bids,
            Side::Ask => &mut self.asks,
        }[ev.level as usize % LEVELS];
        match ev.op {
            Op::Delete => {
                slot.0 = 0.0;
                slot.1 = 0.0;
            }
            Op::Insert | Op::Update => {
                slot.0 = ev.px;
                slot.1 = ev.qty;
            }
        }
    }

    pub fn best_bid(&self) -> f32 {
        self.bids
            .iter()
            .filter(|&&(p, q)| p > 0.0 && q > 0.0)
            .map(|&(p, _)| p)
            .fold(f32::NEG_INFINITY, f32::max)
    }

    pub fn best_ask(&self) -> f32 {
        self.asks
            .iter()
            .filter(|&&(p, q)| p > 0.0 && q > 0.0)
            .map(|&(p, _)| p)
            .fold(f32::INFINITY, f32::min)
    }

    /// zstd-compressed bytes (~1KB)
    fn to_bytes(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(LEVELS * 2 * 8);
        for &(p, q) in self.bids.iter().chain(self.asks.iter()) {
            raw.extend_from_slice(&p.to_le_bytes());
            raw.extend_from_slice(&q.to_le_bytes());
        }
        zstd::encode_all(raw.as_slice(), 3).unwrap()
    }

    fn from_bytes(data: &[u8]) -> Result<Self> {
        let raw = zstd::decode_all(data)?;
        let mut snap = BookSnapshot::default();
        let mut offset = 0usize;
        for i in 0..LEVELS {
            let p = f32::from_le_bytes(raw[offset..offset + 4].try_into()?);
            let q = f32::from_le_bytes(raw[offset + 4..offset + 8].try_into()?);
            snap.bids[i] = (p, q);
            offset += 8;
        }
        for i in 0..LEVELS {
            let p = f32::from_le_bytes(raw[offset..offset + 4].try_into()?);
            let q = f32::from_le_bytes(raw[offset + 4..offset + 8].try_into()?);
            snap.asks[i] = (p, q);
            offset += 8;
        }
        Ok(snap)
    }
}

/// In-memory checkpoint index entry.
#[derive(Clone)]
struct CheckpointMeta {
    ts: i64,
    file_offset: u64,
    snapshot_len: u32,
    event_index: usize, // absolute event index this checkpoint starts at
}

pub struct BookStore {
    _path: PathBuf,
    file: File,
    checkpoints: Vec<CheckpointMeta>,
    ts_start: i64,
    ts_end: i64,
    n_events: u64,
    /// Base px used for current segment delta encoding (last checkpoint px midpoint).
    base_px: f32,
}

impl BookStore {
    pub fn open(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let meta = file.metadata()?;
        let mut store = BookStore {
            _path: path.to_path_buf(),
            file,
            checkpoints: Vec::new(),
            ts_start: 0,
            ts_end: 0,
            n_events: 0,
            base_px: 0.0,
        };
        if meta.len() >= HEADER_SIZE as u64 {
            store.load_index()?;
        } else {
            store.write_header()?;
        }
        Ok(store)
    }

    fn write_header(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut hdr = [0u8; HEADER_SIZE];
        hdr[0..8].copy_from_slice(&MAGIC.to_le_bytes());
        hdr[8..12].copy_from_slice(&VERSION.to_le_bytes());
        // rest zeros
        self.file.write_all(&hdr)?;
        self.file.flush()?;
        Ok(())
    }

    fn load_index(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut hdr = [0u8; HEADER_SIZE];
        self.file.read_exact(&mut hdr)?;
        let magic = u64::from_le_bytes(hdr[0..8].try_into()?);
        anyhow::ensure!(magic == MAGIC, "bad magic");
        self.ts_start = i64::from_le_bytes(hdr[12..20].try_into()?);
        self.ts_end = i64::from_le_bytes(hdr[20..28].try_into()?);
        self.n_events = u64::from_le_bytes(hdr[28..36].try_into()?);
        let n_checkpoints = u64::from_le_bytes(hdr[36..44].try_into()?) as usize;

        // Scan file to rebuild checkpoint list
        let mut pos = HEADER_SIZE as u64;
        let file_len = self.file.metadata()?.len();
        self.checkpoints.clear();
        let mut event_idx = 0usize;

        while pos < file_len {
            self.file.seek(SeekFrom::Start(pos))?;
            let mut tag = [0u8; 4];
            if self.file.read_exact(&mut tag).is_err() {
                break;
            }
            if &tag == b"CKPT" {
                // checkpoint: 4 tag + 8 ts + 4 snap_len + snap_data
                let mut ts_buf = [0u8; 8];
                self.file.read_exact(&mut ts_buf)?;
                let ts = i64::from_le_bytes(ts_buf);
                let mut len_buf = [0u8; 4];
                self.file.read_exact(&mut len_buf)?;
                let snap_len = u32::from_le_bytes(len_buf);
                self.checkpoints.push(CheckpointMeta {
                    ts,
                    file_offset: pos,
                    snapshot_len: snap_len,
                    event_index: event_idx,
                });
                pos += 4 + 8 + 4 + snap_len as u64;
            } else if &tag == b"EVTS" {
                // event block: 4 tag + 4 count + count*12
                let mut cnt_buf = [0u8; 4];
                self.file.read_exact(&mut cnt_buf)?;
                let count = u32::from_le_bytes(cnt_buf) as usize;
                event_idx += count;
                pos += 4 + 4 + (count * 12) as u64;
            } else {
                break;
            }
        }
        let _ = n_checkpoints; // validated implicitly
        Ok(())
    }

    fn flush_header(&mut self) -> Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        let mut hdr = [0u8; HEADER_SIZE];
        hdr[0..8].copy_from_slice(&MAGIC.to_le_bytes());
        hdr[8..12].copy_from_slice(&VERSION.to_le_bytes());
        hdr[12..20].copy_from_slice(&self.ts_start.to_le_bytes());
        hdr[20..28].copy_from_slice(&self.ts_end.to_le_bytes());
        hdr[28..36].copy_from_slice(&self.n_events.to_le_bytes());
        let nc = self.checkpoints.len() as u64;
        hdr[36..44].copy_from_slice(&nc.to_le_bytes());
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&hdr)?;
        Ok(())
    }

    /// Write a checkpoint block at current EOF.
    fn write_checkpoint(&mut self, snap: &BookSnapshot, ts: i64, event_index: usize) -> Result<()> {
        let snap_bytes = snap.to_bytes();
        let pos = self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(b"CKPT")?;
        self.file.write_all(&ts.to_le_bytes())?;
        self.file
            .write_all(&(snap_bytes.len() as u32).to_le_bytes())?;
        self.file.write_all(&snap_bytes)?;
        self.checkpoints.push(CheckpointMeta {
            ts,
            file_offset: pos,
            snapshot_len: snap_bytes.len() as u32,
            event_index,
        });
        Ok(())
    }

    pub fn append(&mut self, events: &[BookEvent]) -> Result<()> {
        if events.is_empty() {
            return Ok(());
        }

        let cur_event_idx = self.n_events as usize;

        // First event: if no checkpoints yet, write initial checkpoint from zeroed snapshot
        if self.checkpoints.is_empty() {
            let snap = BookSnapshot::default();
            self.ts_start = events[0].ts;
            self.write_checkpoint(&snap, events[0].ts, 0)?;
            self.base_px = 0.0;
        }

        // We write events in segments, inserting checkpoints every 1000 events.
        let mut remaining = events;
        let mut idx = cur_event_idx;

        while !remaining.is_empty() {
            let events_in_seg = idx % EVENTS_PER_CHECKPOINT;
            let space = EVENTS_PER_CHECKPOINT - events_in_seg;
            let chunk = &remaining[..remaining.len().min(space)];

            let base_ts = self.checkpoints.last().unwrap().ts;
            let bp = self.base_px;

            // Pack events
            let packed: Vec<PackedEvent> = chunk
                .iter()
                .map(|e| PackedEvent::encode(e, base_ts, bp))
                .collect();

            let count = packed.len() as u32;
            self.file.seek(SeekFrom::End(0))?;
            self.file.write_all(b"EVTS")?;
            self.file.write_all(&count.to_le_bytes())?;
            // SAFETY: PackedEvent is repr(C,packed) POD
            let bytes = unsafe {
                std::slice::from_raw_parts(packed.as_ptr() as *const u8, packed.len() * 12)
            };
            self.file.write_all(bytes)?;

            idx += chunk.len();
            self.n_events += chunk.len() as u64;
            if let Some(last) = chunk.last() {
                self.ts_end = last.ts;
            }

            // If we hit a 1000-event boundary, write checkpoint
            if idx.is_multiple_of(EVENTS_PER_CHECKPOINT) {
                // Rebuild snapshot up to this point for the checkpoint
                let snap = self.build_snapshot_at_idx(idx)?;
                let snap_ts = chunk.last().unwrap().ts;
                self.base_px = snap.best_bid().max(0.0);
                self.write_checkpoint(&snap, snap_ts, idx)?;
            }

            remaining = &remaining[chunk.len()..];
        }

        self.file.flush()?;
        self.flush_header()?;
        self.file.flush()?;
        Ok(())
    }

    /// Read a checkpoint snapshot at given checkpoints vec index.
    fn read_checkpoint_snapshot(&mut self, cp_idx: usize) -> Result<BookSnapshot> {
        let cp = &self.checkpoints[cp_idx];
        let offset = cp.file_offset + 4 + 8 + 4;
        let len = cp.snapshot_len as usize;
        self.file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        self.file.read_exact(&mut buf)?;
        BookSnapshot::from_bytes(&buf)
    }

    /// Reconstruct snapshot by applying all events up to (but not including) `target_event_idx`,
    /// starting from the last checkpoint before it.
    fn build_snapshot_at_idx(&mut self, target_event_idx: usize) -> Result<BookSnapshot> {
        // Find last checkpoint at or before target_event_idx
        let cp_idx = self
            .checkpoints
            .partition_point(|c| c.event_index <= target_event_idx)
            .saturating_sub(1);

        let cp_event_idx = self.checkpoints[cp_idx].event_index;
        let cp_offset = self.checkpoints[cp_idx].file_offset;
        let cp_snap_len = self.checkpoints[cp_idx].snapshot_len;
        let cp_ts = self.checkpoints[cp_idx].ts;

        let mut snap = self.read_checkpoint_snapshot(cp_idx)?;

        // Scan EVTS blocks after this checkpoint, applying up to target_event_idx
        let mut pos = cp_offset + 4 + 8 + 4 + cp_snap_len as u64;
        let mut ev_idx = cp_event_idx;
        let file_len = self.file.metadata()?.len();

        let mut seg_base_ts = cp_ts;
        // base_px used for encoding the segment after this checkpoint equals
        // the best_bid of the checkpoint snapshot (set by append after building snap).
        let mut seg_base_px = snap.best_bid().max(0.0);

        while pos < file_len && ev_idx < target_event_idx {
            self.file.seek(SeekFrom::Start(pos))?;
            let mut tag = [0u8; 4];
            if self.file.read_exact(&mut tag).is_err() {
                break;
            }
            if &tag == b"EVTS" {
                let mut cnt_buf = [0u8; 4];
                self.file.read_exact(&mut cnt_buf)?;
                let count = u32::from_le_bytes(cnt_buf) as usize;
                let to_read = count.min(target_event_idx - ev_idx);
                let byte_count = to_read * 12;
                let mut raw = vec![0u8; byte_count];
                self.file.read_exact(&mut raw)?;
                for chunk in raw.chunks_exact(12) {
                    let packed = unsafe { &*(chunk.as_ptr() as *const PackedEvent) };
                    let ev = packed.decode(seg_base_ts, seg_base_px);
                    snap.apply(&ev);
                }
                ev_idx += to_read;
                pos += 4 + 4 + (count * 12) as u64;
            } else if &tag == b"CKPT" {
                let mut ts_buf = [0u8; 8];
                self.file.read_exact(&mut ts_buf)?;
                seg_base_ts = i64::from_le_bytes(ts_buf);
                let mut len_buf = [0u8; 4];
                self.file.read_exact(&mut len_buf)?;
                let snap_len = u32::from_le_bytes(len_buf) as u64;
                // update seg_base_px from the checkpoint snapshot
                let snap_offset = pos + 4 + 8 + 4;
                self.file.seek(SeekFrom::Start(snap_offset))?;
                let mut sbuf = vec![0u8; snap_len as usize];
                self.file.read_exact(&mut sbuf)?;
                if let Ok(s) = BookSnapshot::from_bytes(&sbuf) {
                    seg_base_px = s.best_bid().max(0.0);
                }
                pos += 4 + 8 + 4 + snap_len;
            } else {
                break;
            }
        }
        Ok(snap)
    }

    pub fn replay_at(&mut self, ts: i64) -> Result<BookSnapshot> {
        // Find the checkpoint just before ts
        let cp_idx = self
            .checkpoints
            .partition_point(|c| c.ts <= ts)
            .saturating_sub(1);
        if self.checkpoints.is_empty() {
            return Ok(BookSnapshot::default());
        }

        let cp_ts = self.checkpoints[cp_idx].ts;
        let cp_snap_len = self.checkpoints[cp_idx].snapshot_len;
        let cp_offset = self.checkpoints[cp_idx].file_offset;

        let mut snap = self.read_checkpoint_snapshot(cp_idx)?;

        // Walk EVTS blocks after this checkpoint, apply events with event.ts <= ts
        let mut pos = cp_offset + 4 + 8 + 4 + cp_snap_len as u64;
        let file_len = self.file.metadata()?.len();
        let mut seg_base_ts = cp_ts;
        let mut seg_base_px = snap.best_bid().max(0.0);

        while pos < file_len {
            self.file.seek(SeekFrom::Start(pos))?;
            let mut tag = [0u8; 4];
            if self.file.read_exact(&mut tag).is_err() {
                break;
            }
            if &tag == b"EVTS" {
                let mut cnt_buf = [0u8; 4];
                self.file.read_exact(&mut cnt_buf)?;
                let count = u32::from_le_bytes(cnt_buf) as usize;
                let mut all_packed = vec![0u8; count * 12];
                self.file.read_exact(&mut all_packed)?;
                let mut past_ts = false;
                for chunk in all_packed.chunks_exact(12) {
                    let packed = unsafe { &*(chunk.as_ptr() as *const PackedEvent) };
                    let ev = packed.decode(seg_base_ts, seg_base_px);
                    if ev.ts > ts {
                        past_ts = true;
                        break;
                    }
                    snap.apply(&ev);
                }
                if past_ts {
                    break;
                }
                pos += 4 + 4 + (count * 12) as u64;
            } else if &tag == b"CKPT" {
                let mut ts_buf = [0u8; 8];
                self.file.read_exact(&mut ts_buf)?;
                let ckpt_ts = i64::from_le_bytes(ts_buf);
                if ckpt_ts > ts {
                    break; // All events in this segment are already past ts
                }
                seg_base_ts = ckpt_ts;
                let mut len_buf = [0u8; 4];
                self.file.read_exact(&mut len_buf)?;
                let snap_len = u32::from_le_bytes(len_buf) as u64;
                let snap_off = pos + 4 + 8 + 4;
                self.file.seek(SeekFrom::Start(snap_off))?;
                let mut sbuf = vec![0u8; snap_len as usize];
                self.file.read_exact(&mut sbuf)?;
                if let Ok(s) = BookSnapshot::from_bytes(&sbuf) {
                    snap = s;
                    seg_base_px = snap.best_bid().max(0.0);
                }
                pos += 4 + 8 + 4 + snap_len;
            } else {
                break;
            }
        }
        Ok(snap)
    }

    pub fn bbo_at(&mut self, ts: i64) -> Result<(f32, f32)> {
        let snap = self.replay_at(ts)?;
        Ok((snap.best_bid(), snap.best_ask()))
    }

    pub fn events_between(&mut self, start: i64, end: i64) -> Result<Vec<BookEvent>> {
        if self.checkpoints.is_empty() {
            return Ok(vec![]);
        }

        let cp_idx = self
            .checkpoints
            .partition_point(|c| c.ts <= start)
            .saturating_sub(1);
        let cp = &self.checkpoints[cp_idx];
        let cp_ts = cp.ts;
        let cp_snap_len = cp.snapshot_len;
        let cp_offset = cp.file_offset;

        let mut pos = cp_offset + 4 + 8 + 4 + cp_snap_len as u64;
        let file_len = self.file.metadata()?.len();
        let mut seg_base_ts = cp_ts;
        let mut seg_base_px = 0.0f32;
        let mut result = Vec::new();

        while pos < file_len {
            self.file.seek(SeekFrom::Start(pos))?;
            let mut tag = [0u8; 4];
            if self.file.read_exact(&mut tag).is_err() {
                break;
            }
            if &tag == b"EVTS" {
                let mut cnt_buf = [0u8; 4];
                self.file.read_exact(&mut cnt_buf)?;
                let count = u32::from_le_bytes(cnt_buf) as usize;
                let mut all_packed = vec![0u8; count * 12];
                self.file.read_exact(&mut all_packed)?;
                let mut past_end = false;
                for chunk in all_packed.chunks_exact(12) {
                    let packed = unsafe { &*(chunk.as_ptr() as *const PackedEvent) };
                    let ev = packed.decode(seg_base_ts, seg_base_px);
                    if ev.ts > end {
                        past_end = true;
                        break;
                    }
                    if ev.ts >= start {
                        result.push(ev);
                    }
                }
                if past_end {
                    break;
                }
                pos += 4 + 4 + (count * 12) as u64;
            } else if &tag == b"CKPT" {
                let mut ts_buf = [0u8; 8];
                self.file.read_exact(&mut ts_buf)?;
                let ckpt_ts = i64::from_le_bytes(ts_buf);
                if ckpt_ts > end {
                    break;
                }
                seg_base_ts = ckpt_ts;
                let mut len_buf = [0u8; 4];
                self.file.read_exact(&mut len_buf)?;
                let snap_len = u32::from_le_bytes(len_buf) as u64;
                let snap_off = pos + 4 + 8 + 4;
                self.file.seek(SeekFrom::Start(snap_off))?;
                let mut sbuf = vec![0u8; snap_len as usize];
                self.file.read_exact(&mut sbuf)?;
                if let Ok(s) = BookSnapshot::from_bytes(&sbuf) {
                    seg_base_px = s.best_bid().max(0.0);
                }
                pos += 4 + 8 + 4 + snap_len;
            } else {
                break;
            }
        }
        Ok(result)
    }

    pub fn n_events(&self) -> u64 {
        self.n_events
    }
    pub fn n_checkpoints(&self) -> usize {
        self.checkpoints.len()
    }
}
