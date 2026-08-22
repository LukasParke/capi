fn cfg() -> capi::cec::Configuration {
    Configuration {
        device_name: "cycle".into(),
        device_type: DeviceType::RECORDING,
        physical_address: 0xFFFF,
        base_device: LogicalAddress::TV,
        hdmi_port: 1,
        monitor_only: false,
        activate_source: false,
        wake_devices: vec![],
        power_off_devices: vec![],
    }
}
use capi::cec::{Configuration, DeviceType, LogicalAddress};
fn main() {
    let n: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    for i in 0..n {
        let c = capi::cec::Connection::open(&cfg()).expect("open");
        println!("cycle {i}: opened");
        c.close().expect("close");
        println!("cycle {i}: closed ok");
    }
}
