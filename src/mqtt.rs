//! MQTT bridge: subscribes {prefix}/command/# and republishes hub events.
//!
//! Fixes vs Go: availability topic with LWT ({prefix}/status retained
//! online/offline), QoS1 publishes, command handlers spawned off the event
//! loop thread (slow CEC commands never stall message processing).

use crate::types::AppEvent;
use rumqttc::{AsyncClient, Event as MqttEvent, MqttOptions, Packet, QoS};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

/// One inbound MQTT command, dispatched by the app.
#[derive(Debug)]
pub struct MqttCommand {
    /// Suffix under {prefix}/command/, e.g. "power/on", "key".
    pub action: String,
    pub payload: Vec<u8>,
}

pub type CommandTx = mpsc::UnboundedSender<MqttCommand>;

#[derive(Clone)]
pub struct MqttHandle {
    session: Arc<Mutex<Option<MqttSession>>>,
    connected: Arc<AtomicBool>,
}

struct MqttSession {
    client: AsyncClient,
    prefix: String,
    loop_task: tokio::task::JoinHandle<()>,
    pub_task: tokio::task::JoinHandle<()>,
}

impl Default for MqttHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttHandle {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// (Re)start the bridge; tears down any previous session. Safe to call
    /// with an empty broker (no-op / stop).
    pub fn start(
        &self,
        cfg: crate::types::MqttConfig,
        mut events: broadcast::Receiver<AppEvent>,
        command_tx: CommandTx,
    ) {
        self.stop();
        if cfg.broker.is_empty() {
            return;
        }
        let prefix = cfg.prefix.clone();
        let hostport = cfg
            .broker
            .strip_prefix("tcp://")
            .or_else(|| cfg.broker.strip_prefix("ssl://"))
            .unwrap_or(&cfg.broker)
            .to_string();
        let (host, port) = match hostport.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p.parse().unwrap_or(1883)),
            None => (hostport.clone(), 1883),
        };

        let mut opts = MqttOptions::new(format!("capi-{}", std::process::id()), host, port);
        opts.set_keep_alive(Duration::from_secs(30));
        if !cfg.user.is_empty() {
            opts.set_credentials(&cfg.user, &cfg.pass);
        }
        opts.set_last_will(rumqttc::LastWill::new(
            format!("{prefix}/status"),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));

        let (client, mut eventloop) = AsyncClient::new(opts, 64);
        let connected = self.connected.clone();
        let cmd_prefix = prefix.clone();
        let cmd_tx = command_tx.clone();
        let client2 = client.clone();
        let client3 = client2.clone();

        let loop_task = tokio::spawn(async move {
            let mut subscribed = false;
            loop {
                match eventloop.poll().await {
                    Ok(MqttEvent::Incoming(Packet::ConnAck(_))) => {
                        connected.store(true, Ordering::Relaxed);
                        subscribed = false;
                    }
                    Ok(MqttEvent::Incoming(Packet::Publish(p))) => {
                        let action = match p.topic.strip_prefix(&format!("{cmd_prefix}/command/")) {
                            Some(a) => a.to_string(),
                            None => continue,
                        };
                        let tx = cmd_tx.clone();
                        let payload = p.payload.to_vec();
                        // Never block the event loop on slow CEC commands.
                        let _ = tx.send(MqttCommand { action, payload });
                    }
                    Ok(MqttEvent::Outgoing(rumqttc::Outgoing::Subscribe(_))) => {
                        subscribed = true;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("mqtt eventloop: {e}");
                        connected.store(false, Ordering::Relaxed);
                        subscribed = false;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
                if connected.load(Ordering::Relaxed) && !subscribed {
                    let _ = client
                        .subscribe(format!("{cmd_prefix}/command/#"), QoS::AtLeastOnce)
                        .await;
                    let _ = client
                        .publish(
                            format!("{cmd_prefix}/status"),
                            QoS::AtLeastOnce,
                            true,
                            "online",
                        )
                        .await;
                }
            }
        });

        let pub_prefix = prefix.clone();
        let connected2 = self.connected.clone();

        let pub_task = tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(ev) => {
                        if !connected2.load(Ordering::Relaxed) {
                            continue;
                        }
                        let topic = format!("{pub_prefix}/event/{}", ev.kind);
                        let payload = serde_json::to_vec(&ev.data).unwrap_or_default();
                        let _ = client2
                            .publish(&topic, QoS::AtLeastOnce, false, payload)
                            .await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("mqtt publisher lagged, dropped {n}");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });

        *self.session.lock().expect("mqtt lock") = Some(MqttSession {
            client: client3,
            prefix,
            loop_task,
            pub_task,
        });
    }

    pub fn stop(&self) {
        let session = self.session.lock().expect("mqtt lock").take();
        if let Some(s) = session {
            s.loop_task.abort();
            s.pub_task.abort();
            let client = s.client;
            let status = format!("{}/status", s.prefix);
            tokio::spawn(async move {
                let _ = client
                    .publish(&status, QoS::AtLeastOnce, true, "offline")
                    .await;
                let _ = client.disconnect().await;
            });
        }
        self.connected.store(false, Ordering::Relaxed);
    }
}
