//! Bus steward: bounded queue of light/full/deep rebuild jobs executed on a
//! dedicated thread, plus a debounced topology-hint worker fed by bus traffic.
//! All slow libcec calls live here — never on request paths.

use crate::busstate::BusState;
use crate::events::EventHub;
use crate::types::Config;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Light,
    Full,
    Deep,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Light => "light",
            JobKind::Full => "full",
            JobKind::Deep => "deep",
        }
    }
}

pub struct StewardJob {
    pub kind: JobKind,
    pub done: Option<tokio::sync::oneshot::Sender<()>>,
}

const QUEUE_CAP: usize = 32;

static MONITORING: AtomicBool = AtomicBool::new(false);

pub fn set_monitoring(on: bool) {
    MONITORING.store(on, Ordering::Relaxed);
}

fn monitoring() -> bool {
    MONITORING.load(Ordering::Relaxed)
}

pub struct Steward {
    tx: SyncSender<StewardJob>,
    hint_tx: std::sync::mpsc::Sender<bool>,
    queued: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl Steward {
    /// Test/stand-in constructor: no worker threads; enqueue/hint are no-ops.
    pub fn detached() -> Arc<Steward> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            // Drain and drop: keeps senders happy without executing jobs.
            while rx.recv().is_ok() {}
        });
        let (hint_tx, hint_rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || while hint_rx.recv().is_ok() {});
        Arc::new(Self {
            tx,
            hint_tx,
            queued: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Spawn the worker threads.
    pub fn spawn(
        bus: Arc<BusState>,
        hub: Arc<EventHub>,
        settings: Arc<crate::settings::Settings>,
        conn: crate::adapter::AdapterHandle,
        metrics_queued: Arc<AtomicU64>,
        metrics_dropped: Arc<AtomicU64>,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<StewardJob>(QUEUE_CAP);
        let last_interval = Arc::new(std::sync::Mutex::new(Duration::from_secs(60)));

        // Main worker: executes jobs and enqueues periodic full reconciles.
        {
            let bus = bus.clone();
            let hub = hub.clone();
            let settings = settings.clone();
            let conn = conn.clone();
            let tx_out = tx.clone();
            let last_interval = last_interval.clone();
            let q_queued = metrics_queued.clone();
            let q_dropped = metrics_dropped.clone();
            std::thread::Builder::new()
                .name("steward".into())
                .spawn(move || {
                    let mut last_tick = Instant::now();
                    loop {
                        let interval = *last_interval.lock().expect("interval lock");
                        let timeout = interval.saturating_sub(last_tick.elapsed());
                        match rx.recv_timeout(timeout) {
                            Ok(job) => {
                                run_job(&job, &bus, &hub, &settings, &conn);
                                if let Some(d) = job.done {
                                    let _ = d.send(());
                                }
                                q_queued.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                        if last_tick.elapsed() >= interval {
                            last_tick = Instant::now();
                            if let Err(e) = tx_out.try_send(StewardJob {
                                kind: JobKind::Full,
                                done: None,
                            }) {
                                warn!("steward queue full, periodic reconcile skipped: {e}");
                                q_dropped.fetch_add(1, Ordering::Relaxed);
                            }
                            *last_interval.lock().expect("interval lock") =
                                settings.get().bus.reconcile_interval_std();
                        }
                    }
                })
                .expect("spawn steward");
        }

        // Topology-hint worker: debounces traffic hints into steward jobs.
        let (hint_tx, hint_rx) = std::sync::mpsc::channel::<bool>();
        {
            let tx_out = tx.clone();
            let q_dropped = metrics_dropped.clone();
            std::thread::Builder::new()
                .name("topology-hints".into())
                .spawn(move || {
                    const DEBOUNCE: Duration = Duration::from_millis(500);
                    loop {
                        let Ok(first) = hint_rx.recv() else { return };
                        let mut heavy = first;
                        std::thread::sleep(DEBOUNCE);
                        while let Ok(more) = hint_rx.try_recv() {
                            heavy |= more;
                        }
                        let kind = if heavy { JobKind::Full } else { JobKind::Light };
                        if let Err(TrySendError::Full(_)) =
                            tx_out.try_send(StewardJob { kind, done: None })
                        {
                            q_dropped.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
                .expect("spawn topology-hints");
        }

        Self {
            tx,
            hint_tx,
            queued: metrics_queued.clone(),
            dropped: metrics_dropped.clone(),
        }
    }

    /// Debounced topology hint from bus traffic; heavy escalates pending light.
    pub fn hint(&self, heavy: bool) {
        let _ = self.hint_tx.send(heavy);
    }

    pub fn enqueue(&self, kind: JobKind) -> bool {
        self.tx.try_send(StewardJob { kind, done: None }).is_ok()
    }

    /// Enqueue and wait up to `timeout` for completion.
    pub async fn enqueue_wait(&self, kind: JobKind, timeout: Duration) -> Result<(), StewardWait> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .try_send(StewardJob {
                kind,
                done: Some(tx),
            })
            .map_err(|_| StewardWait::QueueFull)?;
        tokio::time::timeout(timeout, rx)
            .await
            .map_err(|_| StewardWait::Timeout)?
            .map_err(|_| StewardWait::Timeout)
    }

    pub fn counters(&self) -> (u64, u64) {
        (
            self.queued.load(Ordering::Relaxed),
            self.dropped.load(Ordering::Relaxed),
        )
    }
}

#[derive(Debug)]
pub enum StewardWait {
    QueueFull,
    Timeout,
}

fn run_job(
    job: &StewardJob,
    bus: &Arc<BusState>,
    hub: &Arc<EventHub>,
    settings: &Arc<crate::settings::Settings>,
    conn: &crate::adapter::AdapterHandle,
) {
    let cfg: Config = settings.get();
    let Some(c) = conn.get() else {
        bus.set_cec_ready(false);
        bus.set_scan_in_progress(false);
        return;
    };

    bus.set_scan_in_progress(true);
    bus.bump_generation();

    match job.kind {
        JobKind::Deep | JobKind::Full => {
            let mut settle = Duration::from_secs(1);
            if job.kind == JobKind::Deep {
                settle = cfg.bus.deep_settle();
            }
            settle += cfg.bus.rescan_extra_settle();
            if let Err(e) = c.rescan_devices(settle) {
                tracing::warn!("rescan_devices: {e:#}");
            }
        }
        JobKind::Light => {}
    }

    bus.prune_stale_observed(cfg.bus.stale_device_ttl());

    let Ok(active) = c.logical_addresses_with_poll(true) else {
        bus.set_scan_in_progress(false);
        return;
    };
    let active_set: std::collections::HashSet<i32> = active.iter().map(|a| a.0 as i32).collect();
    let active_src = c.get_active_source().map(|a| a.0 as i32).unwrap_or(-1);

    let deadline = Instant::now() + Duration::from_secs(25);
    let mut devices: Vec<serde_json::Value> = Vec::with_capacity(active.len() + 8);
    for addr in &active {
        if Instant::now() > deadline {
            break;
        }
        match c.get_device_info(*addr) {
            Ok(info) => {
                let mut m = info.to_map();
                m.insert(
                    "polled_at".into(),
                    serde_json::json!(chrono::Utc::now().to_rfc3339()),
                );
                m.insert("discovery".into(), serde_json::json!("active"));
                devices.push(serde_json::Value::Object(m));
            }
            Err(_) => continue,
        }
    }

    if job.kind == JobKind::Deep {
        run_give_probes(&c, &devices, &cfg);
    }

    // Ghost devices: ever-observed initiators missing from libcec's mask.
    for addr in bus.observed_addresses() {
        if active_set.contains(&addr) {
            continue;
        }
        devices.push(serde_json::json!({
            "logical_address": addr,
            "address_name": crate::cec::logical_address_name(addr as u8),
            "device_type": crate::cec::device_type_for_address(addr as u8),
            "discovery": "observed",
        }));
    }

    let (first, last) = bus.seen_timestamps();
    for d in devices.iter_mut() {
        if let Some(obj) = d.as_object_mut() {
            if let Some(la) = obj.get("logical_address").and_then(|v| v.as_i64()) {
                if let Some(t) = first.get(&(la as i32)) {
                    obj.insert("first_seen_at".into(), serde_json::json!(t.to_rfc3339()));
                }
                if let Some(t) = last.get(&(la as i32)) {
                    obj.insert("last_seen_at".into(), serde_json::json!(t.to_rfc3339()));
                }
            }
        }
    }

    let all_addrs: Vec<i32> = devices
        .iter()
        .filter_map(|d| d.get("logical_address").and_then(|v| v.as_i64()))
        .map(|v| v as i32)
        .collect();

    bus.merge_observed_into_devices(&mut devices);
    bus.replace_snapshot(
        devices,
        all_addrs.clone(),
        active_src,
        true,
        monitoring(),
        Some(chrono::Utc::now()),
        cfg.bus.stale_threshold_sec.max(1),
        cfg.bus.frame_ring_size(),
    );

    hub.publish(crate::types::AppEvent::new(
        crate::types::event_type::DEVICES_CHANGED,
        serde_json::json!({
            "reason": "steward",
            "kind": job.kind.as_str(),
            "logical_addresses": all_addrs,
        }),
    ));
    info!("reconcile kind={} addrs={all_addrs:?}", job.kind.as_str());
}

/// Deep-scan Give* probes honoring vendor profiles.
fn run_give_probes(
    c: &std::sync::Arc<crate::cec::Connection>,
    devices: &[serde_json::Value],
    cfg: &Config,
) {
    use crate::cec::{LogicalAddress, Opcode};
    for d in devices {
        let Some(la) = d.get("logical_address").and_then(|v| v.as_i64()) else {
            continue;
        };
        let vendor_key = d
            .get("vendor_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        let profile = cfg.bus.vendor_profiles.get(&vendor_key);
        if let Some(p) = profile {
            if p.settle_ms > 0 {
                std::thread::sleep(Duration::from_millis(p.settle_ms as u64));
            }
        }
        let probes: [(Opcode, &str); 5] = [
            (Opcode::GIVE_DEVICE_VENDOR_ID, "vendor"),
            (Opcode::GIVE_OSD_NAME, "osd"),
            (Opcode::GET_CEC_VERSION, "cec_version"),
            (Opcode::GIVE_DEVICE_POWER_STATUS, "power"),
            (Opcode::GIVE_PHYSICAL_ADDRESS, "physical"),
        ];
        for (op, name) in probes {
            if profile
                .map(|p| p.skip_probes.iter().any(|s| s == name))
                .unwrap_or(false)
            {
                continue;
            }
            let _ = c.transmit(&crate::cec::Command {
                initiator: c
                    .first_logical_address()
                    .unwrap_or(LogicalAddress::FREE_USE),
                destination: LogicalAddress(la as u8),
                opcode: op,
                opcode_set: true,
                parameters: Vec::new(),
                ack: false,
                eom: true,
            });
            std::thread::sleep(Duration::from_millis(120));
        }
    }
}
