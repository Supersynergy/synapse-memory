use anyhow::Result;
use clap::{Parser, Subcommand};
use synapse_rank::{RankConfig, train::train_via_python};
use synapse_learn::query_log::QueryLog;

#[derive(Parser)]
#[command(name = "synapse-rank-train", about = "LambdaMART training for Synapse")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Export query_log.db → LibSVM file for LightGBM
    Export {
        #[arg(long, default_value = "")]
        db: String,
        #[arg(long, default_value = "train.libsvm")]
        out: String,
    },
    /// Train LambdaMART model via python subprocess
    Train {
        #[arg(long, default_value = "train.libsvm")]
        data: String,
        #[arg(long, default_value = "model.lgbm")]
        model: String,
        #[arg(long, default_value = "300")]
        trees: u32,
        #[arg(long, default_value = "0.05")]
        lr: f64,
        #[arg(long, default_value = "8")]
        depth: i32,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.cmd {
        Cmd::Export { db, out } => {
            let db_path = if db.is_empty() {
                QueryLog::default_path()
            } else {
                db.into()
            };
            let ql = QueryLog::open(&db_path)?;
            let n = ql.export_libsvm(&out)?;
            println!("Exported {n} rows → {out}");
        }
        Cmd::Train { data, model, trees, lr, depth } => {
            let cfg = RankConfig {
                num_trees: trees,
                learning_rate: lr,
                max_depth: depth,
                model_path: model,
                libsvm_path: data,
                ..Default::default()
            };
            train_via_python(&cfg)?;
        }
    }
    Ok(())
}
