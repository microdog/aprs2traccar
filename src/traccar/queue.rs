//! Bounded in-memory queue between ingestion and the forwarder; when full,
//! the oldest observation is dropped.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::Notify;
use tokio::time::Instant;
use tracing::warn;

use super::Observation;

/// Minimum interval between warnings about dropped observations.
const DROP_WARNING_INTERVAL: Duration = Duration::from_secs(60);

pub struct Queue {
    state: Mutex<State>,
    available: Notify,
}

struct State {
    items: VecDeque<Observation>,
    capacity: usize,
    closed: bool,
    /// Drops not yet reported in a warning.
    unreported_drops: u64,
    last_drop_warning: Option<Instant>,
}

impl Queue {
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(State {
                items: VecDeque::with_capacity(capacity.min(1024)),
                capacity,
                closed: false,
                unreported_drops: 0,
                last_drop_warning: None,
            }),
            available: Notify::new(),
        }
    }

    /// Appends an observation, dropping the oldest one when full.
    pub fn push(&self, observation: Observation) {
        let mut state = self.lock();
        if state.items.len() >= state.capacity {
            state.items.pop_front();
            state.unreported_drops += 1;
            let now = Instant::now();
            if state
                .last_drop_warning
                .is_none_or(|last| now.duration_since(last) >= DROP_WARNING_INTERVAL)
            {
                warn!(
                    dropped = state.unreported_drops,
                    capacity = state.capacity,
                    "Traccar queue full; dropped the oldest reports"
                );
                state.unreported_drops = 0;
                state.last_drop_warning = Some(now);
            }
        }
        state.items.push_back(observation);
        drop(state);
        self.available.notify_one();
    }

    /// Waits for the next observation; `None` once closed and empty.
    pub async fn pop(&self) -> Option<Observation> {
        loop {
            {
                let mut state = self.lock();
                if let Some(observation) = state.items.pop_front() {
                    return Some(observation);
                }
                if state.closed {
                    return None;
                }
            }
            // A notification sent since the check above is kept as a permit.
            self.available.notified().await;
        }
    }

    /// Stops [`pop`](Self::pop) from waiting once the queue is empty.
    pub fn close(&self) {
        self.lock().closed = true;
        self.available.notify_one();
    }

    pub fn len(&self) -> usize {
        self.lock().items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // The state stays consistent even if a holder panicked.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(device_id: &str) -> Observation {
        Observation {
            device_id: device_id.to_owned(),
            time: jiff::Timestamp::UNIX_EPOCH,
            fix: None,
            attributes: Vec::new(),
        }
    }

    #[tokio::test]
    async fn drops_the_oldest_when_full() {
        let queue = Queue::new(2);
        for id in ["A", "B", "C"] {
            queue.push(observation(id));
        }
        queue.close();
        assert_eq!(queue.pop().await.unwrap().device_id, "B");
        assert_eq!(queue.pop().await.unwrap().device_id, "C");
        assert_eq!(queue.pop().await, None);
    }

    #[tokio::test]
    async fn pop_waits_for_push_and_close() {
        let queue = std::sync::Arc::new(Queue::new(4));
        let consumer = tokio::spawn({
            let queue = queue.clone();
            async move {
                let mut ids = Vec::new();
                while let Some(observation) = queue.pop().await {
                    ids.push(observation.device_id);
                }
                ids
            }
        });
        tokio::task::yield_now().await;
        queue.push(observation("A"));
        tokio::task::yield_now().await;
        queue.push(observation("B"));
        queue.close();
        assert_eq!(consumer.await.unwrap(), ["A", "B"]);
    }
}
