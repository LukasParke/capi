//! CEC session supervisor: owns the connection lifecycle — open libcec, find
//! and open the adapter, publish to the shared handle, drain events through
//! the app callback, reconnect on signal or backoff.

use crate::adapter::{AdapterHandle, WaitReason};
use crate::busstate::BusState;
use crate::events::EventHub;
use crate::settings::Settings;
use crate::steward;
use crate::types::{AppEvent, Config};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

pub static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

const MIN_BACKOFF: Duration = Duration::from_secs(3);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub type CecEventSink =
    Arc<dyn Fn(&Arc<crate::cec::Connection>, crate::cec::CecEvent) + Send + Sync>;

pub struct SupervisorDeps {
    pub settings: Arc<Settings>,
    pub adapter: AdapterHandle,
    pub bus: Arc<BusState>,
    pub hub: Arc<EventHub>,
}

/// Runs on a dedicated thread (all libcec calls are blocking). `on_event`
/// receives every CEC event from the live session.
pub fn run_supervisor(
    deps: SupervisorDeps,
    device_name: String,
    requested_adapter: String,
    monitor_from_flag: bool,
    on_event: CecEventSink,
) {
    let mut backoff = MIN_BACKOFF;
    loop {
        if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
            return;
        }
        let cfg: Config = deps.settings.get();
        let cec_cfg = crate::cec::Configuration {
            device_name: device_name.clone(),
            device_type: crate::cec::DeviceType::RECORDING,
            physical_address: 0xFFFF,
            base_device: crate::cec::LogicalAddress::TV,
            hdmi_port: 1,
            monitor_only: cfg.cec.monitor_only,
            activate_source: cfg.cec.activate_source,
            wake_devices: clamp_addrs(&cfg.cec.wake_on_connect),
            power_off_devices: clamp_addrs(&cfg.cec.power_off_on_disconnect),
        };

        let conn = match crate::cec::Connection::open(&cec_cfg) {
            Ok(c) => Arc::new(c),
            Err(e) => {
                error!("session start failed: {e:#} (retry in {backoff:?})");
                if !sleep_std(backoff) {
                    return;
                }
                backoff = next_backoff(backoff);
                continue;
            }
        };

        let picked = if !requested_adapter.is_empty() {
            Ok(requested_adapter.clone())
        } else {
            match conn.find_adapters() {
                Ok(list) if !list.is_empty() => {
                    let first = &list[0];
                    Ok(if first.comm.starts_with("/dev/") {
                        first.comm.clone()
                    } else {
                        first.path.clone()
                    })
                }
                Ok(_) => Err("no CEC adapter found".to_string()),
                Err(e) => Err(format!("find_adapters: {e:#}")),
            }
        };
        let path = match picked {
            Ok(p) => p,
            Err(e) => {
                warn!("{e}");
                let _ = conn.close();
                if !sleep_std(backoff) {
                    return;
                }
                backoff = next_backoff(backoff);
                continue;
            }
        };

        info!("opening CEC adapter: {path}");
        if let Err(e) = conn.open_adapter(&path) {
            error!("open_adapter({path}): {e:#}");
            let _ = conn.close();
            if !sleep_std(backoff) {
                return;
            }
            backoff = next_backoff(backoff);
            continue;
        }

        backoff = MIN_BACKOFF;

        let mut monitor = monitor_from_flag;
        if let Some(v) = cfg.bus.monitor {
            monitor = v;
        }
        if let Err(e) = conn.switch_monitoring(monitor) {
            warn!("switch_monitoring({monitor}): {e:#}");
        }
        steward::set_monitoring(monitor);
        deps.bus.set_frame_ring_capacity(cfg.bus.frame_ring_size());

        deps.adapter.set(Some(conn.clone()));
        deps.bus.set_cec_ready(true);
        info!("adapter session ready (monitor={monitor})");
        publish_state(&deps.hub, "connected");

        // Event consumer for this session.
        let mut rx = conn.subscribe_events();
        let consumer_conn = conn.clone();
        let on_event = on_event.clone();
        std::thread::Builder::new()
            .name("cec-events".into())
            .spawn(move || loop {
                match rx.blocking_recv() {
                    Ok(ev) => on_event(&consumer_conn, ev),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("cec event consumer lagged, dropped {n}");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            })
            .expect("spawn cec-events");

        match deps
            .adapter
            .wait_for(&SHUTDOWN_FLAG, Duration::from_secs(u64::MAX / 2))
        {
            WaitReason::Timeout => continue,
            reason => {
                publish_state(&deps.hub, "disconnected");
                deps.adapter.set(None);
                deps.bus.set_cec_ready(false);
                steward::set_monitoring(false);
                deps.bus.set_frame_ring_capacity(0);
                let _ = conn.close();
                match reason {
                    WaitReason::Shutdown => return,
                    WaitReason::Reconnect => {
                        info!("reconnect requested; tearing down adapter session");
                        if !sleep_std(Duration::from_secs(1)) {
                            return;
                        }
                    }
                    WaitReason::Timeout => unreachable!(),
                }
            }
        }
    }
}

fn publish_state(hub: &Arc<EventHub>, state: &str) {
    hub.publish(AppEvent::new(
        crate::types::event_type::ADAPTER_STATE,
        serde_json::json!({ "state": state }),
    ));
}

/// Shutdown-aware sleep for supervisor threads.
pub fn sleep_std(d: Duration) -> bool {
    let deadline = std::time::Instant::now() + d;
    while std::time::Instant::now() < deadline {
        if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(
            Duration::from_millis(100)
                .min(deadline.saturating_duration_since(std::time::Instant::now())),
        );
    }
    !SHUTDOWN_FLAG.load(Ordering::SeqCst)
}

fn next_backoff(cur: Duration) -> Duration {
    (cur * 2).min(MAX_BACKOFF)
}

fn clamp_addrs(v: &[i32]) -> Vec<u8> {
    v.iter()
        .filter(|x| **x >= 0 && **x <= 14)
        .map(|x| *x as u8)
        .collect()
}
