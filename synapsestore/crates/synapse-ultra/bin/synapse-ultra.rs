use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use synapse_ultra::cache::T0Cache;
use synapse_ultra::embed::Embedder;
use synapse_ultra::http::{serve, AppState};
use synapse_ultra::index::load_or_rebuild;

#[derive(Parser)]
#[command(
    name = "synapse-ultra",
    about = "High-throughput Synapse vector search daemon"
)]
struct Args {
    /// brain.db path
    #[arg(long, default_value = "~/.synapse/brain.db")]
    brain: String,

    /// Snapshot cache path
    #[arg(long, default_value = "~/.synapse/ultra_matrix.bin")]
    snap: String,

    /// emb_cache.db path
    #[arg(long, default_value = "~/.synapse/emb_cache.db")]
    emb_cache: String,

    /// HTTP bind address
    #[arg(long, default_value = "127.0.0.1:9477")]
    bind: String,

    /// Unix socket path (msgpack, text queries)
    #[arg(long, default_value = "/tmp/synapse-ultra.sock")]
    sock: String,

    /// Vec UDS path (bincode, pre-computed f32 vectors)
    #[arg(long, default_value = "/tmp/synapse-ultra-vec.sock")]
    vec_sock: String,

    /// Pre-warm embedder at startup (downloads model if needed)
    #[arg(long, default_value_t = false)]
    warm: bool,

    /// LRU cache capacity
    #[arg(long, default_value_t = 32768)]
    cache_cap: usize,
}

fn expand_tilde(s: &str) -> PathBuf {
    if s.starts_with("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(format!("{}/{}", home, &s[2..]))
    } else {
        PathBuf::from(s)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("synapse_ultra=info".parse()?))
        .init();

    let args = Args::parse();
    let brain = expand_tilde(&args.brain);
    let snap = expand_tilde(&args.snap);
    let emb_cache = expand_tilde(&args.emb_cache);

    tracing::info!("loading index from {:?}", brain);
    let index = load_or_rebuild(&brain, &snap)?;

    let embedder = Arc::new(Embedder::new(Some(&emb_cache))?);
    if args.warm {
        tracing::info!("warming embedder...");
        embedder.embed("warmup query")?;
    }

    let cache = Arc::new(T0Cache::new(args.cache_cap));

    let state = Arc::new(AppState {
        index: Arc::clone(&index),
        cache: Arc::clone(&cache),
        embedder: Arc::clone(&embedder),
    });

    let sock_path = args.sock.clone();
    let sock_index = Arc::clone(&index);
    let sock_cache = Arc::clone(&cache);
    let sock_emb = Arc::clone(&embedder);

    tokio::spawn(async move {
        if let Err(e) =
            synapse_ultra::socket::serve(&sock_path, sock_index, sock_cache, sock_emb).await
        {
            tracing::error!("socket server error: {}", e);
        }
    });

    let vec_sock_path = args.vec_sock.clone();
    let vec_sock_index = Arc::clone(&index);
    tokio::spawn(async move {
        if let Err(e) = synapse_ultra::vec_socket::serve(&vec_sock_path, vec_sock_index).await {
            tracing::error!("vec socket server error: {}", e);
        }
    });

    serve(state, &args.bind).await?;
    Ok(())
}
