//! Sony controller support through the WinRT HID API.
//!
//! `gilrs` reads Windows pads through `Windows.Gaming.Input`. That stack
//! depends on the `XBOX Synthetic Gamepad Bus` device, and it enumerates no
//! controllers at all when that bus fails to start, which happens when a HID
//! filter driver is attached above it. Reading the DualSense HID interface
//! directly keeps Sony pads working in that situation, and it needs no
//! administrator rights, driver changes, or `unsafe` code.
//!
//! Only Sony pads are handled here; Xbox pads still come from `gilrs`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::HumanInterfaceDevice::{HidDevice, HidInputReportReceivedEventArgs};
use windows::Foundation::TypedEventHandler;
use windows::Storage::FileAccessMode;
use windows::Storage::Streams::DataReader;
use windows::core::{HSTRING, Result};

use crate::gamepad::{PadState, parse_sony_report};

/// Vendor ID shared by Sony Interactive Entertainment controllers.
const SONY_VENDOR_ID: u16 = 0x054C;
/// `DualSense` and `DualSense Edge` share one input report layout.
const PRODUCT_IDS: [u16; 2] = [0x0CE6, 0x0DF2];
/// Generic desktop controls, game pad.
const USAGE_PAGE: u16 = 0x01;
const USAGE_ID: u16 = 0x05;
/// Fastest rate at which a missing pad is looked for.
const MIN_SCAN_INTERVAL: Duration = Duration::from_secs(1);
/// Slowest rate at which a missing pad is looked for.
///
/// Enumeration touches the device stack, so a machine with no Sony pad should
/// not pay for a scan every second.
const MAX_SCAN_INTERVAL: Duration = Duration::from_secs(8);
/// A pad that stops reporting for this long is treated as disconnected.
const REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// Latest report, written by the HID callback and read by the game loop.
#[derive(Default)]
struct Shared {
    state: PadState,
    reports: u64,
}

/// One opened HID interface plus the bookkeeping needed to notice a disconnect.
struct SonyPad {
    name: String,
    device: HidDevice,
    /// Kept alive so the platform keeps the subscription registered.
    _handler: TypedEventHandler<HidDevice, HidInputReportReceivedEventArgs>,
    token: i64,
    shared: Arc<Mutex<Shared>>,
    reports: u64,
    last_report: Instant,
}

impl SonyPad {
    /// Whether the pad has stopped delivering reports.
    fn is_stale(&self) -> bool {
        self.last_report.elapsed() >= REPORT_TIMEOUT
    }

    /// Copy the latest report, refreshing the liveness timestamp on change.
    fn state(&mut self) -> PadState {
        let (reports, state) = match self.shared.lock() {
            Ok(shared) => (shared.reports, shared.state),
            Err(poisoned) => {
                let shared = poisoned.into_inner();
                (shared.reports, shared.state)
            }
        };
        if reports != self.reports {
            self.reports = reports;
            self.last_report = Instant::now();
        }
        state
    }
}

impl Drop for SonyPad {
    fn drop(&mut self) {
        let _ = self.device.RemoveInputReportReceived(self.token);
    }
}

/// Sony pad reader for the Windows HID interface.
///
/// Scanning is throttled, so polling never blocks on a device that is not
/// present. `disabled` never scans at all, which keeps headless tests free of
/// device access.
pub struct SonyHid {
    pad: Option<SonyPad>,
    /// When to look for a pad again; `None` disables scanning.
    next_scan: Option<Instant>,
    /// Current delay between failed scans, doubled on every miss.
    backoff: Duration,
}

impl SonyHid {
    /// Create a reader that scans for a pad on the first poll.
    pub fn new() -> Self {
        Self {
            pad: None,
            next_scan: Some(Instant::now()),
            backoff: MIN_SCAN_INTERVAL,
        }
    }

    /// A reader that never touches a device.
    pub fn disabled() -> Self {
        Self {
            pad: None,
            next_scan: None,
            backoff: MAX_SCAN_INTERVAL,
        }
    }

    /// Return the latest pad state, opening or reopening a pad when needed.
    pub fn poll(&mut self) -> Option<PadState> {
        let stale = self.pad.as_ref().is_some_and(SonyPad::is_stale);
        if stale {
            if let Some(pad) = self.pad.take() {
                log::info!(
                    target: "fate.combat",
                    "Sony controller stopped reporting: {}",
                    pad.name
                );
            }
            self.backoff = MIN_SCAN_INTERVAL;
            self.next_scan = Some(Instant::now() + MIN_SCAN_INTERVAL);
        }

        if self.pad.is_none()
            && let Some(next) = self.next_scan
            && Instant::now() >= next
        {
            let started = Instant::now();
            let opened = self.try_open();
            log::debug!(
                target: "fate.combat",
                "Sony controller scan took {:?}",
                started.elapsed()
            );

            // Enumeration touches the device stack, so slow the retry down while
            // no pad answers instead of scanning on every interval.
            self.backoff = if opened {
                MIN_SCAN_INTERVAL
            } else {
                self.backoff.saturating_mul(2).min(MAX_SCAN_INTERVAL)
            };
            self.next_scan = Some(Instant::now() + self.backoff);
        }

        self.pad.as_mut().map(SonyPad::state)
    }

    /// Open the first Sony pad that exposes a game pad usage.
    ///
    /// Returns whether a pad was opened.
    fn try_open(&mut self) -> bool {
        for product_id in PRODUCT_IDS {
            let Ok(selector) = HidDevice::GetDeviceSelectorVidPid(
                USAGE_PAGE,
                USAGE_ID,
                SONY_VENDOR_ID,
                product_id,
            ) else {
                continue;
            };
            let Ok(list) = DeviceInformation::FindAllAsyncAqsFilter(&selector)
                .and_then(|operation| operation.join())
            else {
                continue;
            };

            let count = list.Size().unwrap_or(0);
            for index in 0..count {
                let Ok(info) = list.GetAt(index) else {
                    continue;
                };
                let Ok(id) = info.Id() else {
                    continue;
                };
                let name = info
                    .Name()
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default();

                match open_pad(&id, name) {
                    Ok(pad) => {
                        log::info!(
                            target: "fate.combat",
                            "controller connected: {} (read through the HID fallback)",
                            pad.name
                        );
                        self.pad = Some(pad);
                        return true;
                    }
                    Err(error) => log::debug!(
                        target: "fate.combat",
                        "could not open Sony controller: {error}"
                    ),
                }
            }
        }
        false
    }
}

/// Open one HID interface and subscribe to its input reports.
fn open_pad(id: &HSTRING, name: String) -> Result<SonyPad> {
    let device = HidDevice::FromIdAsync(id, FileAccessMode::Read)?.join()?;

    let shared = Arc::new(Mutex::new(Shared::default()));
    let sink = Arc::clone(&shared);
    let handler = TypedEventHandler::<HidDevice, HidInputReportReceivedEventArgs>::new(
        move |_sender, args| {
            let Ok(args) = args.ok() else {
                return Ok(());
            };
            let Ok(report) = args.Report() else {
                return Ok(());
            };
            let report_id = u8::try_from(report.Id().unwrap_or(0)).unwrap_or(0);
            let Ok(buffer) = report.Data() else {
                return Ok(());
            };
            let Ok(reader) = DataReader::FromBuffer(&buffer) else {
                return Ok(());
            };

            let mut bytes = vec![0u8; reader.UnconsumedBufferLength().unwrap_or(0) as usize];
            if reader.ReadBytes(&mut bytes).is_ok()
                && let Some(state) = parse_sony_report(report_id, &bytes)
            {
                let mut shared = match sink.lock() {
                    Ok(shared) => shared,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if shared.reports == 0 {
                    log::debug!(
                        target: "fate.combat",
                        "Sony controller report stream started (id {report_id:#04x}, {} bytes, \
                         sticks ({:+.2}, {:+.2}) ({:+.2}, {:+.2}), triggers {:.2}/{:.2})",
                        bytes.len(),
                        state.left_x,
                        state.left_y,
                        state.right_x,
                        state.right_y,
                        state.left_trigger,
                        state.right_trigger,
                    );
                }
                shared.state = state;
                shared.reports = shared.reports.wrapping_add(1);
            }
            Ok(())
        },
    );

    let token = device.InputReportReceived(&handler)?;

    Ok(SonyPad {
        name,
        device,
        _handler: handler,
        token,
        shared,
        reports: 0,
        last_report: Instant::now(),
    })
}
