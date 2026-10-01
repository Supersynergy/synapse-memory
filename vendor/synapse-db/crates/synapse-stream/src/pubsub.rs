use crate::cdc::ChangeEvent;
use tokio::sync::broadcast;

pub struct Hub {
    tx: broadcast::Sender<ChangeEvent>,
}

impl Hub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, ev: ChangeEvent) {
        let _ = self.tx.send(ev);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.tx.subscribe()
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new(4096)
    }
}
