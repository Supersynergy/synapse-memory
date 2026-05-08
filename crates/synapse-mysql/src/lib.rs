//! synapse-mysql — async MySQL wire proxy backed by `opensrv-mysql` v0.10.
//!
//! Replaces the prior `msql_srv` blocking sync impl that capped at ~700 OPS
//! (8 thread, workload C) with tokio async (target ≥10k OPS).
//!
//! See `docs/SYNAPSE_VS_MYSQL_LIMITS.md` for the gap analysis this closes.

use std::io;
use async_trait::async_trait;
use opensrv_mysql::{
    AsyncMysqlIntermediary, AsyncMysqlShim, OkResponse, ParamParser,
    QueryResultWriter, StatementMetaWriter,
};
use tokio::io::AsyncWrite;
use tokio::net::TcpListener;

/// Pluggable backend: implement to delegate SQL to a real `Store`.
#[async_trait]
pub trait QueryBackend: Send + Sync + 'static {
    async fn on_query(&self, sql: &str) -> Result<QueryResult, BackendError>;
}

#[derive(Debug, Default)]
pub struct QueryResult {
    pub affected: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("backend: {0}")]
    Other(String),
}

/// Minimal stub backend — echoes SQL, returns empty resultset.
/// Replace with `synapse-core::Store` adapter once wire glue lands.
pub struct EchoBackend;

#[async_trait]
impl QueryBackend for EchoBackend {
    async fn on_query(&self, _sql: &str) -> Result<QueryResult, BackendError> {
        Ok(QueryResult::default())
    }
}

pub struct ShimAdapter<B: QueryBackend> {
    pub backend: B,
}

#[async_trait]
impl<W, B> AsyncMysqlShim<W> for ShimAdapter<B>
where
    W: AsyncWrite + Send + Unpin,
    B: QueryBackend,
{
    type Error = io::Error;

    async fn on_prepare<'a>(
        &'a mut self,
        _: &'a str,
        info: StatementMetaWriter<'a, W>,
    ) -> io::Result<()> {
        info.reply(0, &[], &[]).await
    }

    async fn on_execute<'a>(
        &'a mut self,
        _: u32,
        _: ParamParser<'a>,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        results.completed(OkResponse::default()).await
    }

    async fn on_close(&mut self, _: u32) {}

    async fn on_query<'a>(
        &'a mut self,
        sql: &'a str,
        results: QueryResultWriter<'a, W>,
    ) -> io::Result<()> {
        let _ = self
            .backend
            .on_query(sql)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        results.start(&[]).await?.finish().await
    }
}

/// Bind & accept connections forever. One tokio task per connection.
pub async fn serve<B>(addr: &str, make_backend: impl Fn() -> B + Send + Sync + 'static) -> io::Result<()>
where
    B: QueryBackend,
{
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let shim = ShimAdapter { backend: make_backend() };
        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            let _ = AsyncMysqlIntermediary::run_on(shim, r, w).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_backend_returns_empty_result() {
        let r = EchoBackend.on_query("SELECT 1").await.unwrap();
        assert_eq!(r.affected, 0);
    }
}
