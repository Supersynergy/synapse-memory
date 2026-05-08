//! synapse-pg — async Postgres wire proxy via `pgwire` v0.40.
//!
//! Pattern from `repos/pgwire/examples/gluesql.rs`. Cluster-G of synapse-gap-sprint.

use std::sync::Arc;
use async_trait::async_trait;
use futures::stream;
use std::fmt::Debug;
use futures::Sink;
use pgwire::api::query::SimpleQueryHandler;
use pgwire::api::results::{FieldInfo, QueryResponse, Response};
use pgwire::api::{ClientInfo, ClientPortalStore, PgWireServerHandlers};
use pgwire::api::store::PortalStore;
use pgwire::messages::data::DataRow;
use pgwire::error::PgWireResult;
use pgwire::messages::PgWireBackendMessage;
use pgwire::tokio::process_socket;
use tokio::net::TcpListener;

#[async_trait]
pub trait QueryBackend: Send + Sync + 'static {
    async fn on_query(&self, sql: &str) -> Result<u64, BackendError>;
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("backend: {0}")]
    Other(String),
}

pub struct EchoBackend;

#[async_trait]
impl QueryBackend for EchoBackend {
    async fn on_query(&self, _sql: &str) -> Result<u64, BackendError> {
        Ok(0)
    }
}

pub struct SynapsePgProcessor<B: QueryBackend> {
    pub backend: Arc<B>,
}

#[async_trait]
impl<B: QueryBackend> SimpleQueryHandler for SynapsePgProcessor<B> {
    async fn do_query<C>(
        &self,
        _client: &mut C,
        query: &str,
    ) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
        C::Error: Debug,
        pgwire::error::PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        self.backend
            .on_query(query)
            .await
            .map_err(|e| pgwire::error::PgWireError::ApiError(Box::new(e)))?;
        let fields: Arc<Vec<FieldInfo>> = Arc::new(vec![]);
        let stream = stream::iter(Vec::<PgWireResult<DataRow>>::new());
        Ok(vec![Response::Query(QueryResponse::new(fields, stream))])
    }
}

pub struct Factory<B: QueryBackend> {
    pub processor: Arc<SynapsePgProcessor<B>>,
}

impl<B: QueryBackend> PgWireServerHandlers for Factory<B> {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        self.processor.clone()
    }
}

pub async fn serve<B: QueryBackend>(addr: &str, backend: Arc<B>) -> std::io::Result<()> {
    let factory = Arc::new(Factory {
        processor: Arc::new(SynapsePgProcessor { backend }),
    });
    let listener = TcpListener::bind(addr).await?;
    loop {
        let (stream, _) = listener.accept().await?;
        let f = factory.clone();
        tokio::spawn(async move {
            let _ = process_socket(stream, None, f).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn echo_backend() {
        assert_eq!(EchoBackend.on_query("SELECT 1").await.unwrap(), 0);
    }
}
