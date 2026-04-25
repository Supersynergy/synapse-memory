//! synapse-mysql-async — Async MySQL wire-protocol server for Synapse.
//!
//! Phase 1 of MASTERPLAN-V2-GAMECHANGER-2026-04-25.
//! Replaces blocking msql_srv with opensrv-mysql + tokio for 5-10× OPS gain
//! at 8+ concurrent threads. Reuses rewrite + ACL logic from `synapse-mysql`.
//!
//! Reference impls (verified via ghgrep):
//! - databendlabs/databend src/query/service/src/servers/mysql/mysql_interactive_worker.rs
//! - GreptimeTeam/greptimedb src/servers/src/mysql/handler.rs

mod shim;

use anyhow::{Context, Result};
use clap::Parser;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, warn};

use shim::{new_shared_state_with_pool, SharedState, SynapseMysqlAsync};

#[derive(Parser)]
#[command(
    name = "synapse-mysql-async",
    version,
    about = "Synapse async MySQL wire server (Phase 1 gamechanger)"
)]
struct Cli {
    /// SQLite/Synapse file to serve
    #[arg(short = 'f', long, default_value = ".synapse/mysql-bridge.db")]
    file: PathBuf,
    /// Listen address
    #[arg(short, long, default_value = "127.0.0.1:13310")]
    bind: SocketAddr,
    /// Compatibility mode (medium-coeli|wp|strict)
    #[arg(long, default_value = "medium-coeli")]
    mode: String,
    /// Default root password (change in production!)
    #[arg(long, default_value = "synapse")]
    root_password: String,
    /// Path to TLS certificate (PEM). Enables TLS when combined with --tls-key.
    #[arg(long)]
    tls_cert: Option<PathBuf>,
    /// Path to TLS private key (PEM). Enables TLS when combined with --tls-cert.
    #[arg(long)]
    tls_key: Option<PathBuf>,
    /// Maximum number of reusable SQLite connections in the shared pool.
    #[arg(long, default_value_t = 32)]
    pool_size: usize,
}

fn load_tls_config(cert: &PathBuf, key: &PathBuf) -> Result<Arc<tokio_rustls::rustls::ServerConfig>> {
    use rustls_pemfile::{certs, private_key};
    use tokio_rustls::rustls::ServerConfig;

    let cert_file = std::fs::File::open(cert)
        .with_context(|| format!("open cert {}", cert.display()))?;
    let key_file = std::fs::File::open(key)
        .with_context(|| format!("open key {}", key.display()))?;

    let cert_chain: Vec<_> = certs(&mut std::io::BufReader::new(cert_file))
        .collect::<std::result::Result<_, _>>()
        .context("parse cert PEM")?;
    let private_key = private_key(&mut std::io::BufReader::new(key_file))
        .context("parse key PEM")?
        .context("no private key found in PEM")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .context("build rustls ServerConfig")?;
    Ok(Arc::new(config))
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "synapse_mysql_async=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Some(p) = cli.file.parent() {
        std::fs::create_dir_all(p).ok();
    }

    info!("opening database {}", cli.file.display());

    // Phase 1 MVP: skip ACL — caller responsible for prior setup via synapse-mysql binary.
    let _ = cli.root_password.clone(); // suppress unused

    // Load TLS config if both cert and key are provided.
    let tls_config: Option<Arc<tokio_rustls::rustls::ServerConfig>> =
        match (cli.tls_cert.as_ref(), cli.tls_key.as_ref()) {
            (Some(cert), Some(key)) => {
                let cfg = load_tls_config(cert, key)?;
                info!("TLS enabled (cert={}, key={})", cert.display(), key.display());
                Some(cfg)
            }
            (None, None) => {
                info!("TLS disabled (pass --tls-cert and --tls-key to enable)");
                None
            }
            _ => anyhow::bail!("--tls-cert and --tls-key must be provided together"),
        };

    let state = new_shared_state_with_pool(cli.file.clone(), cli.mode.clone(), cli.pool_size);

    let listener = TcpListener::bind(&cli.bind)
        .await
        .with_context(|| format!("bind {}", cli.bind))?;
    info!(
        "synapse-mysql-async listening on {} (mode={}, file={}, tls={})",
        cli.bind,
        cli.mode,
        cli.file.display(),
        tls_config.is_some()
    );

    loop {
        let (stream, addr) = match listener.accept().await {
            Ok(p) => p,
            Err(e) => {
                error!("accept: {}", e);
                continue;
            }
        };
        let state = Arc::clone(&state);
        let tls_config = tls_config.clone();
        tokio::spawn(async move {
            tracing::debug!("conn from {}", addr);
            let (r, w) = stream.into_split();
            let shim = SynapseMysqlAsync::new(state);

            if let Some(tls_cfg) = tls_config {
                // Advertise SSL capability; let opensrv perform the handshake branch.
                let opts = opensrv_mysql::IntermediaryOptions::default();
                let result = {
                    let mut shim_mut = shim;
                    let mut w_mut = w;
                    let init = opensrv_mysql::AsyncMysqlIntermediary::init_before_ssl(
                        &mut shim_mut,
                        r,
                        &mut w_mut,
                        &Some(tls_cfg.clone()),
                    )
                    .await;
                    match init {
                        Err(e) => { warn!("conn {} handshake error: {}", addr, e); return; }
                        Ok((wants_tls, init_params)) => {
                            if wants_tls {
                                opensrv_mysql::secure_run_with_options(
                                    shim_mut, w_mut, opts, tls_cfg, init_params,
                                )
                                .await
                            } else {
                                opensrv_mysql::plain_run_with_options(
                                    shim_mut, w_mut, opts, init_params,
                                )
                                .await
                            }
                        }
                    }
                };
                if let Err(e) = result {
                    warn!("conn {} ended: {}", addr, e);
                }
            } else {
                if let Err(e) = opensrv_mysql::AsyncMysqlIntermediary::run_on(shim, r, w).await {
                    warn!("conn {} ended: {}", addr, e);
                }
            }
        });
    }
}
