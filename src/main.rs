//! capi — HDMI-CEC control over HTTP and MQTT (Rust). Binary entry point;
//! all wiring lives in `capi::server::run` so integration tests can drive
//! the full stack in-process.

fn main() -> std::process::ExitCode {
    let flags = match capi::settings::parse_flags(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "capi: {e}\nusage: capi [-bind :8080] [-name N] [-adapter PATH] [-token T]\n             [-mqtt-broker URL] [-mqtt-user U] [-mqtt-pass P] [-mqtt-prefix P]\n             [-cec-monitor] [-version] [-update]"
            );
            return std::process::ExitCode::from(2);
        }
    };
    if flags.show_version {
        println!("{}", capi::ui::VERSION);
        return std::process::ExitCode::SUCCESS;
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let code = rt.block_on(capi::server::run(flags));
    std::process::ExitCode::from(code)
}
