use capi::*;

#[allow(unused_imports)]
#[test]
fn envelope_shapes() {
    let ok = types::Envelope::success("m", Some(serde_json::json!(1)));
    assert_eq!(ok.status, "success");
    let err = types::Envelope::error("bad");
    assert_eq!(err.status, "error");
    assert!(err.message.contains("bad"));
}

#[test]
fn config_defaults_and_bus_accessors() {
    let c = types::Config::default();
    assert_eq!(c.mqtt.prefix, "capi");
    assert_eq!(c.bus.frame_ring_size(), 256);
    assert_eq!(
        c.bus.stale_device_ttl(),
        std::time::Duration::from_secs(600)
    );
    let mut b = types::BusConfig {
        frame_ring_size: -1,
        ..types::BusConfig::default()
    };
    assert_eq!(b.frame_ring_size(), 0);
    b.reconcile_interval_sec = 5;
    assert_eq!(
        b.reconcile_interval_std(),
        std::time::Duration::from_secs(5)
    );
    b.deep_settle_ms = 100;
    assert_eq!(b.deep_settle(), std::time::Duration::from_millis(100));
    b.stale_threshold_sec = 30;
    assert_eq!(b.stale_threshold(), std::time::Duration::from_secs(30));
}

#[test]
fn vendor_profile_defaults() {
    let vp: types::VendorProfile = serde_json::from_str("{}").unwrap();
    assert!(vp.skip_probes.is_empty());
    assert_eq!(vp.settle_ms, 0);
}

#[test]
fn adapter_handle_lifecycle() {
    let a = AdapterHandle::new();
    assert!(!a.ready());
    a.signal_reconnect();
    match a.wait_for(
        &supervisor::SHUTDOWN_FLAG,
        std::time::Duration::from_millis(50),
    ) {
        adapter::WaitReason::Reconnect => {}
        other => panic!("expected reconnect, got {other:?}"),
    }
    // Second wait: nothing pending -> Timeout.
    match a.wait_for(
        &supervisor::SHUTDOWN_FLAG,
        std::time::Duration::from_millis(50),
    ) {
        adapter::WaitReason::Timeout => {}
        other => panic!("expected timeout, got {other:?}"),
    }
    supervisor::SHUTDOWN_FLAG.store(true, std::sync::atomic::Ordering::SeqCst);
    match a.wait_for(
        &supervisor::SHUTDOWN_FLAG,
        std::time::Duration::from_secs(10),
    ) {
        adapter::WaitReason::Shutdown => {}
        other => panic!("expected shutdown, got {other:?}"),
    }
    supervisor::SHUTDOWN_FLAG.store(false, std::sync::atomic::Ordering::SeqCst);
}

#[test]
fn topology_port_parsing_edges() {
    assert_eq!(capi::topology::port_of("F.F.F.F"), None);
}

#[test]
fn settings_overlay_token() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = Settings::load(&dir.path().join("c.json")).unwrap();
    s.apply_overrides(&settings::CliOverrides {
        mqtt_broker: None,
        mqtt_user: Some("u".into()),
        mqtt_pass: Some("p".into()),
        mqtt_prefix_explicit: true,
        mqtt_prefix: "custom".into(),
        token: None,
    });
    let c = s.get();
    assert_eq!(c.mqtt.user, "u");
    assert_eq!(c.mqtt.prefix, "custom");
}
