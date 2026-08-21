//! Event hub (broadcast), log ring, and metrics counters.

use crate::types::AppEvent;
use chrono::Utc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Fan-out for app events. Tokio broadcast channels cannot panic on send:
/// with zero receivers `send` returns Err (counted), with slow receivers the
/// receiver observes `Lagged` — the Go drop-counter semantics, without the
/// send-on-closed-channel crash class.
pub struct EventHub {
    tx: broadcast::Sender<AppEvent>,
    pub dropped: AtomicU64,
    pub delivered: AtomicU64,
}

impl EventHub {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self {
            tx,
            dropped: AtomicU64::new(0),
            delivered: AtomicU64::new(0),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }

    pub fn publish(&self, ev: AppEvent) {
        match self.tx.send(ev) {
            Ok(n) => {
                self.delivered.fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }

    pub fn stats(&self) -> (u64, u64) {
        (
            self.dropped.load(Ordering::Relaxed),
            self.delivered.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LogMessage {
    pub timestamp: String,
    pub level: String, // "CEC" | "ERROR" | "WARN" | "INFO" | "DEBUG"
    pub message: String,
}

/// In-memory ring of recent log lines (surfaced at /api/logs + UI).
pub struct LogRing {
    inner: Mutex<Vec<LogMessage>>,
    cap: usize,
}

impl LogRing {
    pub fn new(cap: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Vec::with_capacity(cap)),
            cap,
        })
    }

    pub fn push(&self, level: &str, message: String) {
        let mut g = self.inner.lock().expect("log lock");
        if g.len() >= self.cap {
            g.remove(0);
        }
        g.push(LogMessage {
            timestamp: Utc::now().to_rfc3339(),
            level: level.to_string(),
            message,
        });
    }

    pub fn recent(&self) -> Vec<LogMessage> {
        self.inner.lock().expect("log lock").clone()
    }
}

/// Process counters exposed at /metrics.
#[derive(Default)]
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub errors_total: AtomicU64,
    pub panics_total: AtomicU64,
    #[allow(dead_code)]
    pub events_published: AtomicU64,
    #[allow(dead_code)]
    pub steward_jobs_queued: AtomicU64,
    #[allow(dead_code)]
    pub steward_jobs_dropped: AtomicU64,
}
