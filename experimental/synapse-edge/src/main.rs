use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use http::StatusCode;
use pingora_core::prelude::*;
use pingora_core::server::Server;
use pingora_proxy::{ProxyHttp, Session};

const UPSTREAM_HOST: &str = "127.0.0.1";
const UPSTREAM_PORT: u16 = 9477;

struct EdgeProxy;

#[async_trait]
impl ProxyHttp for EdgeProxy {
    type CTX = ();

    fn new_ctx(&self) -> Self::CTX {}

    async fn request_filter(
        &self,
        session: &mut Session,
        _ctx: &mut Self::CTX,
    ) -> pingora_core::Result<bool> {
        if session.req_header().uri.path() == "/health"
            && session.req_header().method == http::Method::GET
        {
            let mut resp = pingora_http::ResponseHeader::build(StatusCode::OK, Some(2))?;
            resp.insert_header("content-type", "text/plain")?;
            resp.insert_header("content-length", "2")?;
            session
                .write_response_header(Box::new(resp), false)
                .await?;
            session
                .write_response_body(Some(Bytes::from_static(b"ok")), true)
                .await?;
            return Ok(true); // short-circuit, do not proxy
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        _ctx: &mut (),
    ) -> pingora_core::Result<Box<HttpPeer>> {
        let path = session.req_header().uri.path();
        match path {
            "/embed" | "/search" | "/hybrid" => {
                let peer = HttpPeer::new(
                    (UPSTREAM_HOST, UPSTREAM_PORT),
                    false,
                    UPSTREAM_HOST.to_string(),
                );
                Ok(Box::new(peer))
            }
            _ => Err(pingora_core::Error::new(pingora_core::ErrorType::HTTPStatus(404))),
        }
    }

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut pingora_http::RequestHeader,
        _ctx: &mut Self::CTX,
    ) -> pingora_core::Result<()> {
        upstream_request
            .insert_header("X-Forwarded-By", "synapse-edge")
            .ok();
        Ok(())
    }

    fn fail_to_connect(
        &self,
        _session: &mut Session,
        _peer: &HttpPeer,
        _ctx: &mut Self::CTX,
        e: Box<pingora_core::Error>,
    ) -> Box<pingora_core::Error> {
        e
    }
}

fn main() -> Result<()> {
    // Install ring as the default TLS crypto provider (required by rustls 0.23+)
    let _ = rustls::crypto::ring::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let port: u16 = std::env::var("SYNAPSE_EDGE_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9478);

    let bind_addr = format!("127.0.0.1:{port}");
    tracing::info!(
        "synapse-edge listening on {bind_addr} → proxying to {UPSTREAM_HOST}:{UPSTREAM_PORT}"
    );

    let mut server = Server::new(None)?;
    server.bootstrap();

    let mut proxy = pingora_proxy::http_proxy_service(&server.configuration, EdgeProxy);
    proxy.add_tcp(&bind_addr);

    server.add_service(proxy);
    server.run_forever();
}
