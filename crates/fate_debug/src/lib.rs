//! Opt-in native diagnostics presentation for engine consumers.
//!
//! The host owns its event loop and all application state. It publishes bounded
//! telemetry and inspection data, forwards events for this window, and executes
//! the returned commands. No window or device is created until `DebugWindow::new`.

mod companion;
mod logging;
mod model;
mod ui;
mod window;

pub use companion::DebugCompanion;
pub use logging::{LogInbox, capture_logs};
pub use model::{
    Command, Control, ControlValue, DebugModel, EntityRow, EventLevel, Fact, FrameSample,
    GpuSample, Inspection, Page, Statistics,
};
pub use window::DebugWindow;
