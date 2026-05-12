//! synapsql-mysql — MySQL wire-protocol via opensrv-mysql v0.10.

use std::io;
use std::sync::Arc;
use async_trait::async_trait;
use opensrv_mysql::{
    AsyncMysqlIntermediary, AsyncMysqlShim, OkResponse, ParamParser,
    QueryResultWriter, StatementMetaWriter,
};
use tokio::io::AsyncWrite;
use tokio::net::TcpListener;
use synapse_libsql::Store;

pub struct ShimAdapter {
    pub store: Arc<dyn Store>,
}

#[async_trait]
impl<W> AsyncMysqlShim<W> for ShimAdapter
where
    W: AsyncWrite + Send + Unpin,
{
    type Error = io::Error;

    async fn on_prepare<'a>(&'a mut self, _: &'a str, info: StatementMetaWriter<'a, W>) -> io::Result<()> {
        info.reply(0, &[], &[]).await
    }
    async fn on_execute<'a>(&'a mut self, _: u32, _: ParamParser<'a>, results: QueryResultWriter<'a, W>) -> io::Result<()> {
        results.completed(OkResponse::default()).await
    }
    async fn on_close(&mut self, _: u32) {}
    async fn on_query<'a>(&'a mut self, sql: &'a str, results: QueryResultWriter<'a, W>) -> io::Result<()> {
        let _ = self
            .store
            .query(sql)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        results.start(&[]).await?.finish().await
    }
}

pub async fn serve(addr: &str, store: Arc<dyn Store>) -> io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    eprintln!("synapsql-mysql listening on {}", addr);
    loop {
        let (stream, _peer) = listener.accept().await?;
        let s = store.clone();
        tokio::spawn(async move {
            let (r, w) = stream.into_split();
            let _ = AsyncMysqlIntermediary::run_on(ShimAdapter { store: s }, r, w).await;
        });
    }
}
