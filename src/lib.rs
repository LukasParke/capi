//! capi library surface: everything except the binary entry point lives here
//! so integration tests can drive the real router, steward, and dispatchers.

pub mod adapter;
pub mod assets;
pub mod busstate;
pub mod cec;
pub mod dispatch;
pub mod events;
pub mod exec;
pub mod mqtt;
pub mod server;
pub mod settings;
pub mod steward;
pub mod strategies;
pub mod supervisor;
pub mod topology;
pub mod types;
pub mod ui;
pub mod ui_ctx;
pub mod update;
pub mod util;

pub use adapter::AdapterHandle;
pub use busstate::BusState;
pub use events::{EventHub, LogRing, Metrics};
pub use server::AppState;
pub use settings::Settings;
pub use steward::Steward;
pub use strategies::Registry;

/// Re-exported for the supervisor wiring in main.
pub use supervisor::SHUTDOWN_FLAG;
