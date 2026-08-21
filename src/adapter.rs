//! Shared adapter handle: the live connection behind a lock, plus the
//! reconnect signal consumed by the supervisor. Works from sync threads
//! (supervisor) and async handlers alike.

use crate::cec::Connection;
use std::sync::{Arc, Condvar, Mutex, RwLock, Weak};

#[derive(Clone)]
pub struct AdapterHandle {
    inner: Arc<AdapterInner>,
}

struct AdapterInner {
    current: RwLock<Option<Arc<Connection>>>,
    reconnect_pending: Mutex<bool>,
    reconnect_cv: Condvar,
}

impl AdapterHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AdapterInner {
                current: RwLock::new(None),
                reconnect_pending: Mutex::new(false),
                reconnect_cv: Condvar::new(),
            }),
        }
    }

    pub fn set(&self, conn: Option<Arc<Connection>>) {
        *self.inner.current.write().expect("adapter lock") = conn;
    }

    pub fn get(&self) -> Option<Arc<Connection>> {
        self.inner.current.read().expect("adapter lock").clone()
    }

    pub fn ready(&self) -> bool {
        self.get().is_some()
    }

    /// Ask the supervisor to tear down and reopen the session. Coalesces.
    pub fn signal_reconnect(&self) {
        let mut p = self.inner.reconnect_pending.lock().expect("reconnect lock");
        *p = true;
        self.inner.reconnect_cv.notify_all();
    }

    /// Supervisor-side wait: returns Reconnect when signaled, Shutdown when
    /// the process flag is set. Timeout-bounded so shutdown is always seen.
    pub fn wait_for(
        &self,
        shutdown: &std::sync::atomic::AtomicBool,
        timeout: std::time::Duration,
    ) -> WaitReason {
        let deadline = std::time::Instant::now() + timeout;
        let mut p = self.inner.reconnect_pending.lock().expect("reconnect lock");
        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                return WaitReason::Shutdown;
            }
            if *p {
                *p = false;
                return WaitReason::Reconnect;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return WaitReason::Timeout;
            }
            let (guard, _) = self
                .inner
                .reconnect_cv
                .wait_timeout(p, deadline.saturating_duration_since(now))
                .expect("reconnect cv");
            p = guard;
        }
    }

    #[allow(dead_code)]
    pub fn downgrade(&self) -> AdapterWeak {
        AdapterWeak {
            inner: Arc::downgrade(&self.inner),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum WaitReason {
    Shutdown,
    Reconnect,
    Timeout,
}

#[derive(Clone)]
#[allow(dead_code)]
pub struct AdapterWeak {
    inner: Weak<AdapterInner>,
}

#[allow(dead_code)]
impl AdapterWeak {
    pub fn upgrade(&self) -> Option<AdapterHandle> {
        self.inner.upgrade().map(|inner| AdapterHandle { inner })
    }
}
