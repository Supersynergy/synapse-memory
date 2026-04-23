//! synapse-mysql — MySQL wire-protocol compatibility layer for SynapseDB.
//! Runs on port 3306 (default) and proxies/translates MySQL queries to SQLite.

use anyhow::{Context, Result};
use clap::Parser;
use std::net::{SocketAddr, TcpListener};
use std::thread;
use tracing::{error, info, warn};

mod acl;
mod rewrite;
mod server;

use acl::Acl;
use server::SynapseMySql;

#[derive(Parser)]
#[command(name = "synapse-mysql", version, about = "SynapseDB MySQL compatibility server")]
struct Cli {
    /// SQLite/SynapseDB file to serve
    #[arg(short = 'f', long, default_value = ".synapse/mysql-bridge.db")]
    file: std::path::PathBuf,
    /// Listen address
    #[arg(short, long, default_value = "127.0.0.1:3306")]
    bind: SocketAddr,
    /// Compatibility mode
    #[arg(long, default_value = "medium-coeli")]
    mode: String,
    /// Default root password (change in production!)
    #[arg(long, default_value = "synapse")]
    root_password: String,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "synapse_mysql=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Some(p) = cli.file.parent() {
        std::fs::create_dir_all(p).ok();
    }

    info!("opening database {}", cli.file.display());
    let store = synapse_core::Store::open(&cli.file).context("open store")?;

    // Init ACL tables and root user
    let acl = Acl::new(&store);
    acl.init_tables()?;
    if let Err(e) = acl.ensure_root(&cli.root_password) {
        warn!("root init: {}", e);
    }
    drop(store);

    let listener = TcpListener::bind(&cli.bind)
        .with_context(|| format!("bind {}", cli.bind))?;
    info!("synapse-mysql listening on {} (mode={})", cli.bind, cli.mode);

    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                error!("accept: {}", e);
                continue;
            }
        };
        let addr = match stream.peer_addr() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let file = cli.file.clone();
        let mode = cli.mode.clone();
        let root_password = cli.root_password.clone();
        thread::spawn(move || {
            info!("connection from {}", addr);
            let store = match synapse_core::Store::open(&file) {
                Ok(s) => s,
                Err(e) => {
                    warn!("open store for {}: {}", addr, e);
                    return;
                }
            };
            let acl = Acl::new(&store);
            let _ = acl.init_tables();
            let _ = acl.ensure_root(&root_password);
            let shim = match SynapseMySql::new(store, acl, &mode) {
                Ok(s) => s,
                Err(e) => {
                    warn!("shim init for {}: {}", addr, e);
                    return;
                }
            };
            if let Err(e) = msql_srv::MysqlIntermediary::run_on_tcp(shim, stream) {
                warn!("client {} disconnected: {}", addr, e);
            }
        });
    }

    Ok(())
}
