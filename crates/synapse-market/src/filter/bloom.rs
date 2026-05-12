use xxhash_rust::xxh3::xxh3_64_with_seed;

const BITS: usize = 128 * 1024; // 128K bits = 16KB
const BYTES: usize = BITS / 8;  // 16384

pub struct Bloom {
    bits: Box<[u8; BYTES]>,
    pub k: u8,
}

impl Bloom {
    pub fn new() -> Self {
        Self {
            bits: Box::new([0u8; BYTES]),
            k: 3,
        }
    }

    fn bit_positions(&self, key: u64) -> [usize; 3] {
        [
            (xxh3_64_with_seed(&key.to_le_bytes(), 0) as usize) % BITS,
            (xxh3_64_with_seed(&key.to_le_bytes(), 1) as usize) % BITS,
            (xxh3_64_with_seed(&key.to_le_bytes(), 2) as usize) % BITS,
        ]
    }

    pub fn add(&mut self, key: u64) {
        for pos in self.bit_positions(key) {
            self.bits[pos / 8] |= 1 << (pos % 8);
        }
    }

    pub fn contains(&self, key: u64) -> bool {
        self.bit_positions(key)
            .iter()
            .all(|&pos| self.bits[pos / 8] & (1 << (pos % 8)) != 0)
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.bits.to_vec()
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != BYTES {
            return None;
        }
        let mut b = Box::new([0u8; BYTES]);
        b.copy_from_slice(bytes);
        Some(Self { bits: b, k: 3 })
    }
}

impl Default for Bloom {
    fn default() -> Self {
        Self::new()
    }
}
