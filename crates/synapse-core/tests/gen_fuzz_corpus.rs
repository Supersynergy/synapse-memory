use std::{fs, path::PathBuf};
use synapse_core::synx::{
    chunk::{ChunkKind, Codec},
    header::SynxFlags,
    writer::SynxWriter,
};

type SeedChunk<'a> = (ChunkKind, Codec, &'a [u8]);

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap() // crates/
        .parent()
        .unwrap() // workspace root
        .join("fuzz/corpus/synx_deserialize")
}

fn write_seed(name: &str, chunks: &[SeedChunk<'_>]) {
    let dir = corpus_dir();
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    if path.exists() {
        fs::remove_file(&path).unwrap();
    }
    let mut w = SynxWriter::create(&path, SynxFlags::COMPRESSED).unwrap();
    for (kind, codec, data) in chunks {
        w.append(*kind, *codec, data).unwrap();
    }
    w.finish().unwrap();
}

#[test]
fn generate_fuzz_corpus_seeds() {
    write_seed("seed_01_empty.synx", &[]);
    write_seed(
        "seed_02_text_raw.synx",
        &[(ChunkKind::TextBlob, Codec::Raw, b"hello world")],
    );
    write_seed(
        "seed_03_text_zstd.synx",
        &[(
            ChunkKind::TextBlob,
            Codec::Zstd,
            b"compressed text payload for fuzzing seed",
        )],
    );
    write_seed(
        "seed_04_rowbatch.synx",
        &[(
            ChunkKind::RowBatch,
            Codec::Zstd,
            b"{\"id\":1,\"text\":\"row batch payload\"}",
        )],
    );
    write_seed(
        "seed_05_multi.synx",
        &[
            (ChunkKind::TextBlob, Codec::Raw, b"first chunk"),
            (
                ChunkKind::RowBatch,
                Codec::Zstd,
                b"second chunk zstd compressed payload here",
            ),
        ],
    );
    write_seed(
        "seed_06_schema.synx",
        &[(
            ChunkKind::SchemaDef,
            Codec::Raw,
            b"{\"version\":2,\"fields\":[]}",
        )],
    );
    write_seed(
        "seed_07_unicode.synx",
        &[(
            ChunkKind::TextBlob,
            Codec::Raw,
            "Ünïcödé tëxt 日本語 emojis 🚀🔥".as_bytes(),
        )],
    );
    write_seed(
        "seed_08_binary.synx",
        &[(
            ChunkKind::RowBatch,
            Codec::Raw,
            &[0u8, 1, 2, 127, 128, 255, 0xDE, 0xAD, 0xBE, 0xEF],
        )],
    );
    write_seed(
        "seed_09_large.synx",
        &[(ChunkKind::TextBlob, Codec::Zstd, &vec![b'A'; 4096])],
    );
    write_seed(
        "seed_10_many_chunks.synx",
        &[
            (ChunkKind::TextBlob, Codec::Raw, b"chunk a"),
            (ChunkKind::TextBlob, Codec::Raw, b"chunk b"),
            (
                ChunkKind::RowBatch,
                Codec::Zstd,
                b"chunk c with more compressed data content",
            ),
            (
                ChunkKind::SchemaDef,
                Codec::Raw,
                b"{\"schema\":\"d\",\"idx\":4}",
            ),
        ],
    );
    println!("Corpus written to {}", corpus_dir().display());
}
