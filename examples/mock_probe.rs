fn main() {
    let cfg = capi::cec::Configuration {
        device_name: "p".into(),
        device_type: capi::cec::DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: capi::cec::LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: true,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    };
    let c = capi::cec::Connection::open(&cfg).unwrap();
    c.open_adapter("/dev/mock0").unwrap();
    println!("info={:?}", c.get_lib_info());
    println!(
        "power={:?} vendor={:?} osd={:?}",
        c.get_device_power_status(capi::cec::LogicalAddress::TV),
        c.get_device_vendor_id(capi::cec::LogicalAddress(4)),
        c.get_device_osd_name(capi::cec::LogicalAddress(4))
    );
}
