//! smx_mysql_shim — MySQL wire-protocol shim over synapse-market.
//!
//! Supported queries:
//!   SELECT 1 / SELECT @@version / SELECT VERSION()
//!   SELECT * FROM candles WHERE ticker=? AND ts BETWEEN ? AND ?
//!   SELECT corr_matrix(?, ?)        (comma-separated tickers, ts_start, ts_end)
//!
//! Build: cargo build --bin smx_mysql_shim --features smx-mysql
//! Run:   smx_mysql_shim --addr 127.0.0.1:3307 --db /path/to/market.db
//!
//! Smoke: mysql -h 127.0.0.1 -P 3307 -u root --protocol=TCP -e "SELECT * FROM candles LIMIT 5"

#[cfg(not(feature = "smx-mysql"))]
fn main() {
    eprintln!("smx_mysql_shim requires --features smx-mysql");
    std::process::exit(1);
}

#[cfg(feature = "smx-mysql")]
mod inner {
    use async_trait::async_trait;
    use opensrv_mysql::{
        AsyncMysqlIntermediary, AsyncMysqlShim, Column, ColumnFlags, ColumnType, OkResponse,
        ParamParser, QueryResultWriter, StatementMetaWriter,
    };
    use std::io;
    use std::sync::Arc;
    use tokio::io::AsyncWrite;
    use tokio::net::TcpListener;

    use synapse_market::Market;

    const VERSION: &str = "8.0.32-smx-market-1.0";
    type CorrQuery = (Vec<String>, i64, i64);
    type ResultRows = Vec<Vec<String>>;
    type QueryResult = (Vec<String>, ResultRows);

    // -------------------------------------------------------------------------
    // Arg parsing (no extra deps — just env::args)
    // -------------------------------------------------------------------------

    pub struct Config {
        pub addr: String,
        pub db: String,
    }

    impl Config {
        pub fn from_args() -> Self {
            let args: Vec<String> = std::env::args().collect();
            let mut addr = "127.0.0.1:3307".to_owned();
            let mut db = ":memory:".to_owned();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--addr" => {
                        i += 1;
                        addr = args[i].clone();
                    }
                    "--db" => {
                        i += 1;
                        db = args[i].clone();
                    }
                    _ => {}
                }
                i += 1;
            }
            Self { addr, db }
        }
    }

    // -------------------------------------------------------------------------
    // Shim adapter
    // -------------------------------------------------------------------------

    pub struct SmxShim {
        market: Arc<std::sync::Mutex<Market>>,
    }

    impl SmxShim {
        pub fn new(market: Arc<std::sync::Mutex<Market>>) -> Self {
            Self { market }
        }
    }

    // -------------------------------------------------------------------------
    // Query dispatch helpers
    // -------------------------------------------------------------------------

    /// Parse: SELECT * FROM candles WHERE ticker='X' AND ts BETWEEN N AND M
    fn parse_candles_query(sql: &str) -> Option<(String, i64, i64)> {
        let upper = sql.to_ascii_uppercase();
        if !upper.contains("FROM CANDLES") {
            return None;
        }
        // ticker=
        let ticker = extract_string_eq(sql, "ticker")?;
        let (ts_start, ts_end) = extract_between(sql, "ts")?;
        Some((ticker, ts_start, ts_end))
    }

    fn extract_string_eq(sql: &str, col: &str) -> Option<String> {
        let pattern = format!("{col}=");
        let low = sql.to_ascii_lowercase();
        let pos = low.find(pattern.as_str())?;
        let rest = &sql[pos + pattern.len()..].trim_start();
        if rest.starts_with('\'') {
            let inner = rest.trim_start_matches('\'');
            let end = inner.find('\'')?;
            Some(inner[..end].to_owned())
        } else if rest.starts_with('"') {
            let inner = rest.trim_start_matches('"');
            let end = inner.find('"')?;
            Some(inner[..end].to_owned())
        } else {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == ';')
                .unwrap_or(rest.len());
            Some(rest[..end].to_owned())
        }
    }

    fn extract_between(sql: &str, col: &str) -> Option<(i64, i64)> {
        let lower = sql.to_ascii_lowercase();
        let pat = format!("{} between", col.to_ascii_lowercase());
        let pos = lower.find(pat.as_str())?;
        let rest = sql[pos + pat.len()..].trim_start();
        let mut tokens = rest.split_whitespace();
        let a: i64 = tokens.next()?.parse().ok()?;
        let _and = tokens.next()?; // AND
        let b: i64 = tokens.next()?.trim_end_matches(';').parse().ok()?;
        Some((a, b))
    }

    /// Parse: SELECT corr_matrix('AAPL,MSFT', 1000000, 9999999)
    fn parse_corr_query(sql: &str) -> Option<CorrQuery> {
        let upper = sql.to_ascii_uppercase();
        if !upper.contains("CORR_MATRIX(") {
            return None;
        }
        let start = sql.to_ascii_lowercase().find("corr_matrix(")?;
        let inner_start = start + "corr_matrix(".len();
        let inner_end = sql[inner_start..].find(')')?;
        let args_str = &sql[inner_start..inner_start + inner_end];
        let args: Vec<&str> = args_str.split(',').collect();
        if args.len() < 3 {
            return None;
        }
        let tickers_str = args[0].trim().trim_matches('\'').trim_matches('"');
        let ts_start: i64 = args[1].trim().parse().ok()?;
        let ts_end: i64 = args[2].trim().trim_end_matches(')').parse().ok()?;
        let tickers: Vec<String> = tickers_str.split('|').map(|s| s.to_owned()).collect();
        Some((tickers, ts_start, ts_end))
    }

    fn intercept(sql: &str) -> Option<QueryResult> {
        let s = sql.trim().to_ascii_lowercase();
        let s = s.trim_end_matches(';');
        if s == "select 1" || s.starts_with("select 1 ") {
            return Some((vec!["1".into()], vec![vec!["1".into()]]));
        }
        if s.contains("version()") && s.starts_with("select") {
            return Some((vec!["VERSION()".into()], vec![vec![VERSION.into()]]));
        }
        if s.contains("@@") && s.starts_with("select") {
            return Some((vec!["@@variable".into()], vec![vec![VERSION.into()]]));
        }
        if s.starts_with("show databases") {
            return Some((vec!["Database".into()], vec![vec!["synapse_market".into()]]));
        }
        if s.starts_with("show tables") {
            return Some((
                vec!["Tables_in_synapse_market".into()],
                vec![vec!["candles".into()]],
            ));
        }
        None
    }

    fn handle_candles(market: &Market, ticker: &str, ts_start: i64, ts_end: i64) -> QueryResult {
        let cols = vec![
            "ts".into(),
            "open".into(),
            "high".into(),
            "low".into(),
            "close".into(),
            "volume".into(),
        ];
        let rows = market
            .fetch_candles(ticker, ts_start, ts_end)
            .unwrap_or_default()
            .into_iter()
            .map(|(ts, o, h, l, c, v)| {
                vec![
                    ts.to_string(),
                    o.to_string(),
                    h.to_string(),
                    l.to_string(),
                    c.to_string(),
                    v.to_string(),
                ]
            })
            .collect();
        (cols, rows)
    }

    fn handle_corr(market: &Market, tickers: &[String], ts_start: i64, ts_end: i64) -> QueryResult {
        let ticker_refs: Vec<&str> = tickers.iter().map(|s| s.as_str()).collect();
        let cols = tickers.to_vec();
        let cm = market.correlation_matrix(&ticker_refs, ts_start..ts_end);
        match cm {
            Ok(m) => {
                let n = m.n;
                let rows = (0..n)
                    .map(|r| {
                        (0..n)
                            .map(|c| format!("{:.4}", m.data[r * n + c]))
                            .collect()
                    })
                    .collect();
                (cols, rows)
            }
            Err(e) => (vec!["error".into()], vec![vec![e.to_string()]]),
        }
    }

    // -------------------------------------------------------------------------
    // AsyncMysqlShim impl
    // -------------------------------------------------------------------------

    #[async_trait]
    impl<W> AsyncMysqlShim<W> for SmxShim
    where
        W: AsyncWrite + Send + Unpin,
    {
        type Error = io::Error;

        async fn on_prepare<'a>(
            &'a mut self,
            _sql: &'a str,
            info: StatementMetaWriter<'a, W>,
        ) -> io::Result<()> {
            info.reply(1, &[], &[]).await
        }

        async fn on_execute<'a>(
            &'a mut self,
            _id: u32,
            _params: ParamParser<'a>,
            results: QueryResultWriter<'a, W>,
        ) -> io::Result<()> {
            results.completed(OkResponse::default()).await
        }

        async fn on_close(&mut self, _id: u32) {}

        async fn on_query<'a>(
            &'a mut self,
            sql: &'a str,
            results: QueryResultWriter<'a, W>,
        ) -> io::Result<()> {
            if let Some((cols, rows)) = intercept(sql) {
                return write_result(results, cols, rows).await;
            }

            let upper = sql.trim().to_ascii_uppercase();
            if upper.starts_with("SET ") {
                return results.completed(OkResponse::default()).await;
            }

            if let Some((ticker, ts_start, ts_end)) = parse_candles_query(sql) {
                let (cols, rows) = {
                    let market = self.market.lock().unwrap();
                    handle_candles(&market, &ticker, ts_start, ts_end)
                };
                return write_result(results, cols, rows).await;
            }

            if let Some((tickers, ts_start, ts_end)) = parse_corr_query(sql) {
                let (cols, rows) = {
                    let market = self.market.lock().unwrap();
                    handle_corr(&market, &tickers, ts_start, ts_end)
                };
                return write_result(results, cols, rows).await;
            }

            // Fallback: empty result
            write_result(results, vec!["result".into()], vec![]).await
        }
    }

    async fn write_result<W>(
        results: QueryResultWriter<'_, W>,
        col_names: Vec<String>,
        rows: Vec<Vec<String>>,
    ) -> io::Result<()>
    where
        W: AsyncWrite + Send + Unpin,
    {
        let cols: Vec<Column> = col_names
            .iter()
            .map(|name| Column {
                table: String::new(),
                column: name.clone(),
                collen: 65535,
                coltype: ColumnType::MYSQL_TYPE_VAR_STRING,
                colflags: ColumnFlags::empty(),
            })
            .collect();
        let mut rw = results.start(&cols).await?;
        for row in rows {
            for val in &row {
                rw.write_col(val.as_str())?;
            }
            rw.end_row().await?;
        }
        rw.finish().await
    }

    // -------------------------------------------------------------------------
    // Entry
    // -------------------------------------------------------------------------

    pub async fn run() -> anyhow::Result<()> {
        let cfg = Config::from_args();
        let market = if cfg.db == ":memory:" {
            Market::open_in_memory()?
        } else {
            Market::open(&cfg.db)?
        };
        let market = Arc::new(std::sync::Mutex::new(market));

        let listener = TcpListener::bind(&cfg.addr).await?;
        eprintln!("smx_mysql_shim listening on {} (db={})", cfg.addr, cfg.db);

        loop {
            let (stream, peer) = listener.accept().await?;
            eprintln!("connection from {peer}");
            let m = market.clone();
            tokio::spawn(async move {
                let shim = SmxShim::new(m);
                let (r, w) = stream.into_split();
                if let Err(e) = AsyncMysqlIntermediary::run_on(shim, r, w).await {
                    eprintln!("connection closed: {e}");
                }
            });
        }
    }
}

#[cfg(feature = "smx-mysql")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    inner::run().await
}
