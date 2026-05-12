//! synapsql-pg — Postgres wire-protocol via pgwire 0.40.

use std::fmt::Debug;
use std::sync::Arc;
use async_trait::async_trait;
use futures::{stream, Sink};
use pgwire::api::query::SimpleQueryHandler;
use pgwire::api::results::{FieldInfo, QueryResponse, Response};
use pgwire::api::store::PortalStore;
use pgwire::api::{ClientInfo, ClientPortalStore, PgWireServerHandlers};
use pgwire::error::PgWireResult;
use pgwire::messages::data::DataRow;
use pgwire::messages::PgWireBackendMessage;
use pgwire::tokio::process_socket;
use tokio::net::TcpListener;
use synapse_libsql::Store;

pub struct Processor {
    pub store: Arc<dyn Store>,
}

#[async_trait]
impl SimpleQueryHandler for Processor {
    async fn do_query<C>(&self, _client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + ClientPortalStore + Sink<PgWireBackendMessage> + Unpin + Send + Sync,
        C::PortalStore: PortalStore,
        C::Error: Debug,
        pgwire::error::PgWireError: From<<C as Sink<PgWireBackendMessage>>::Error>,
    {
        self.store
            .query(query)
            .await
            .map_err(|e| pgwire::error::PgWireError::ApiError(Box::new(e)))?;
        let fields: Arc<Vec<FieldInfo>> = Arc::new(vec![]);
        let s = stream::iter(Vec::<PgWireResult<DataRow>>::new());
        Ok(vec![Response::Query(QueryResponse::new(fields, s))])
    }
}

pub struct Factory {
    pub processor: Arc<Processor>,
}

impl PgWireServerHandlers for Factory {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        self.processor.clone()
    }
}

pub async fn serve(addr: &str, store: Arc<dyn Store>) -> std::io::Result<()> {
    let factory = Arc::new(Factory {
        processor: Arc::new(Processor { store }),
    });
    let listener = TcpListener::bind(addr).await?;
    eprintln!("synapsql-pg listening on {}", addr);
    loop {
        let (stream, _) = listener.accept().await?;
        let f = factory.clone();
        tokio::spawn(async move {
            let _ = process_socket(stream, None, f).await;
        });
    }
}
