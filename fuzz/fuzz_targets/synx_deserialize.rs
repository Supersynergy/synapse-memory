#![no_main]

use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use synapse_core::synx::{
    chunk::Chunk,
    header::{SynxFooter, SynxHeader, FOOTER_SIZE},
};

fuzz_target!(|data: &[u8]| {
    // Attempt header parse — must not panic
    let mut cur = Cursor::new(data);
    let _ = SynxHeader::read_from(&mut cur);

    // Attempt footer parse from tail (if long enough)
    if data.len() >= FOOTER_SIZE {
        let tail = &data[data.len() - FOOTER_SIZE..];
        let mut cur = Cursor::new(tail);
        let _ = SynxFooter::read_from(&mut cur);
    }

    // Attempt chunk parse at offset 0
    let mut cur = Cursor::new(data);
    if let Ok(chunk) = Chunk::read_from(&mut cur) {
        // Attempt decompression — must not panic
        let _ = chunk.decode();
    }
});
