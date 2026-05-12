use synapse_market::{
    stream::{Parser, TickStream, WebSocketTickStream},
    Market,
};

const BINANCE_WS: &str = "wss://stream.binance.com/ws/btcusdt@trade";
const MAX_TICKS: usize = 100;

#[tokio::main]
async fn main() {
    println!("stream_smoke: connecting to {BINANCE_WS}");
    let mkt = Market::open_in_memory().expect("open market");

    match WebSocketTickStream::connect(BINANCE_WS, Parser::Generic).await {
        Ok(stream) => {
            println!("connected. ingesting {MAX_TICKS} ticks...");
            match mkt.ingest_stream("BTCUSDT", stream, Some(MAX_TICKS)).await {
                Ok(n) => println!("ingested {n} ticks. done."),
                Err(e) => println!("ingest error: {e}"),
            }
        }
        Err(e) => {
            println!("blocked, skip: {e}");
        }
    }
}
