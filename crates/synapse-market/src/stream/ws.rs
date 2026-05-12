use super::{parser::Parser, LiveTick, TickStream};
use crate::Result;
use futures_util::StreamExt;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Live WebSocket tick source.
pub struct WebSocketTickStream {
    pub url: String,
    pub parser: Parser,
    inner: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl WebSocketTickStream {
    /// Connect to `url` and return a ready stream.
    pub async fn connect(url: &str, parser: Parser) -> Result<Self> {
        let (ws, _) = connect_async(url)
            .await
            .map_err(|e| crate::Error::Market(format!("ws connect: {e}")))?;
        Ok(Self {
            url: url.to_string(),
            parser,
            inner: ws,
        })
    }
}

impl TickStream for WebSocketTickStream {
    type Item = LiveTick;

    async fn next_tick(&mut self) -> Option<Self::Item> {
        loop {
            let msg = self.inner.next().await?.ok()?;
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                Message::Close(_) => return None,
                _ => continue,
            };
            if let Some(tick) = self.parser.parse(&text) {
                return Some(tick);
            }
        }
    }
}
