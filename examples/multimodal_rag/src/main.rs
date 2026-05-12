//! Multimodal RAG — Marqo-killer demo
//!
//! Cross-modal CLIP-style search: index images + text captions,
//! then query by text to find images, or by image to find similar images.
//!
//! Default build uses `multimodal-dummy` (deterministic hash embedder,
//! no ONNX download). Swap feature to `clip-jina` for real 1024-d embeddings.
//!
//! # Run
//!   cargo run -- query text "cat"
//!   cargo run -- query text "vehicle"
//!   cargo run -- query image fixtures/img_cat.png

use anyhow::Result;
use image::{GrayImage, Luma, RgbImage, Rgb};
use std::path::{Path, PathBuf};
use std::time::Instant;
use synapse_multimodal::{ClipEmbedder, CrossModalIndex, ModalKind, MultimodalEmbedder};

// ── fixture generator ─────────────────────────────────────────────────────────

/// Generate a small 64×64 PNG. `r/g/b` values encode "semantic content" for
/// the dummy embedder (histogram-based). Real embedder ignores this.
fn make_fixture(dir: &Path, name: &str, r: u8, g: u8, b: u8) -> Result<PathBuf> {
    let path = dir.join(format!("{name}.png"));
    if !path.exists() {
        let img = RgbImage::from_pixel(64, 64, Rgb([r, g, b]));
        img.save(&path)?;
    }
    Ok(path)
}

struct Fixture {
    id: &'static str,
    r: u8, g: u8, b: u8,
    caption: &'static str,
}

const FIXTURES: &[Fixture] = &[
    Fixture { id: "img_cat",    r: 30,  g: 20,  b: 10,  caption: "a cat sitting on a mat" },
    Fixture { id: "img_cat2",   r: 35,  g: 25,  b: 15,  caption: "kitten playing with yarn" },
    Fixture { id: "img_dog",    r: 90,  g: 80,  b: 70,  caption: "a dog running in the park" },
    Fixture { id: "img_car",    r: 200, g: 50,  b: 50,  caption: "red sports car on the road" },
    Fixture { id: "img_sky",    r: 60,  g: 120, b: 220, caption: "blue sky with white clouds" },
    Fixture { id: "img_tree",   r: 40,  g: 160, b: 40,  caption: "tall oak tree in summer" },
    Fixture { id: "img_pizza",  r: 240, g: 180, b: 80,  caption: "delicious cheese pizza" },
    Fixture { id: "img_bike",   r: 180, g: 180, b: 60,  caption: "mountain bike on trail" },
    Fixture { id: "img_ocean",  r: 30,  g: 80,  b: 200, caption: "ocean waves at sunset" },
    Fixture { id: "img_flower", r: 230, g: 80,  b: 180, caption: "pink flower in bloom" },
];

const TEXT_DOCS: &[(&str, &str)] = &[
    ("txt_cats",     "cats and kittens are popular pets"),
    ("txt_vehicles", "cars, trucks, and bikes on the highway"),
    ("txt_nature",   "forests, trees, and blue skies"),
    ("txt_food",     "pizza, burgers, and street food"),
    ("txt_animals",  "dogs, cats, birds — common household pets"),
];

// ── build index ───────────────────────────────────────────────────────────────

fn build_index(fixture_dir: &Path) -> Result<(CrossModalIndex, ClipEmbedder)> {
    let emb = ClipEmbedder::new();
    let mut idx = CrossModalIndex::new(emb.dim());

    // Index images
    for f in FIXTURES {
        let path = make_fixture(fixture_dir, f.id, f.r, f.g, f.b)?;
        idx.add_image(f.id, &path, Some(f.caption), &emb)?;
    }

    // Index text documents
    for (id, text) in TEXT_DOCS {
        idx.add_text(id, text, &emb);
    }

    println!(
        "Index: {} docs ({} images + {} texts)",
        idx.len(),
        FIXTURES.len(),
        TEXT_DOCS.len()
    );
    Ok((idx, emb))
}

// ── query helpers ─────────────────────────────────────────────────────────────

fn print_hits(hits: &[synapse_multimodal::ModalHit]) {
    if hits.is_empty() {
        println!("  (no results)");
        return;
    }
    for (i, h) in hits.iter().enumerate() {
        let kind = match h.kind { ModalKind::Image => "🖼 ", ModalKind::Text => "📄" };
        println!("  #{} [{:.4}] {kind} {}  — {}", i + 1, h.score, h.id, h.content);
    }
}

fn cmd_query_text(query: &str, fixture_dir: &Path) -> Result<()> {
    let (idx, emb) = build_index(fixture_dir)?;
    println!("\nText query: \"{query}\"");
    let t0 = Instant::now();
    let hits = idx.query_text(query, &emb, 3);
    println!("Latency: {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
    print_hits(&hits);
    Ok(())
}

fn cmd_query_image(image_path: &str, fixture_dir: &Path) -> Result<()> {
    let (idx, emb) = build_index(fixture_dir)?;
    let path = Path::new(image_path);
    println!("\nImage query: {}", path.display());
    let t0 = Instant::now();
    let hits = idx.query_image(path, &emb, 3)?;
    println!("Latency: {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
    print_hits(&hits);
    Ok(())
}

fn run_demo(fixture_dir: &Path) -> Result<()> {
    println!("=== Multimodal RAG demo (dummy embedder, no model download) ===\n");
    let (idx, emb) = build_index(fixture_dir)?;

    let queries = ["cat", "vehicle", "nature", "food"];
    for q in &queries {
        println!("\nText query: \"{q}\"");
        let t0 = Instant::now();
        let hits = idx.query_text(q, &emb, 3);
        println!("Latency: {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
        print_hits(&hits);
    }

    // Image → similar images
    let cat_path = make_fixture(fixture_dir, "img_cat", 30, 20, 10)?;
    println!("\nImage query: img_cat.png → similar images");
    let t0 = Instant::now();
    let hits = idx.query_image(&cat_path, &emb, 3)?;
    println!("Latency: {:.2}ms", t0.elapsed().as_secs_f64() * 1000.0);
    print_hits(&hits);

    println!("\n✓ demo complete");
    Ok(())
}

fn main() -> Result<()> {
    // Fixture dir: persistent `fixtures/` next to binary, or tmp for CI
    let fixture_dir = PathBuf::from("fixtures");
    std::fs::create_dir_all(&fixture_dir)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, kind, query] if cmd == "query" && kind == "text" => {
            cmd_query_text(query, &fixture_dir)
        }
        [cmd, kind, path] if cmd == "query" && kind == "image" => {
            cmd_query_image(path, &fixture_dir)
        }
        [] => run_demo(&fixture_dir),
        [cmd] if cmd == "demo" => run_demo(&fixture_dir),
        _ => {
            eprintln!("Usage:");
            eprintln!("  cargo run                         # full demo");
            eprintln!("  cargo run -- query text \"cat\"");
            eprintln!("  cargo run -- query image fixtures/img_cat.png");
            Ok(())
        }
    }
}
