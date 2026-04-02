use godot::prelude::*;
use godot::classes::{RefCounted, IRefCounted};
use simplersble::{Adapter, Peripheral};
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

use crate::ble_device::BLEDevice;

enum ScanState {
    Idle,
    Scanning,
    Done(Vec<Peripheral>),
    Error(String),
}

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct GdBLE {
    base: Base<RefCounted>,
    adapter: Arc<Mutex<Option<Adapter>>>,
    /// Shared tokio runtime — also handed to BLEDevice for notification streaming.
    runtime: Arc<Mutex<Option<Runtime>>>,
    scan_state: Arc<Mutex<ScanState>>,
}

#[godot_api]
impl IRefCounted for GdBLE {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            adapter: Arc::new(Mutex::new(None)),
            runtime: Arc::new(Mutex::new(None)),
            scan_state: Arc::new(Mutex::new(ScanState::Idle)),
        }
    }
}

#[godot_api]
impl GdBLE {
    /// Initialise the Bluetooth adapter and tokio runtime.
    /// Returns true on success.
    #[func]
    fn initialize(&mut self) -> bool {
        // Build tokio runtime for notification stream consumption.
        let runtime = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                godot_error!("GdBLE: Failed to create tokio runtime: {}", e);
                return false;
            }
        };

        // Check adapter availability.
        let adapters = match Adapter::get_adapters() {
            Ok(a) => a,
            Err(e) => {
                godot_error!("GdBLE: get_adapters() failed: {}", e);
                return false;
            }
        };

        if adapters.is_empty() {
            godot_error!("GdBLE: No Bluetooth adapters found");
            return false;
        }

        *self.runtime.lock().unwrap() = Some(runtime);
        *self.adapter.lock().unwrap() = Some(adapters.into_iter().next().unwrap());

        godot_print!("GdBLE: Bluetooth LE initialized successfully");
        true
    }

    /// Start a non-blocking background scan.
    /// Returns false if a scan is already running.
    /// Poll is_scan_done() / is_scan_error() each frame.
    /// Call take_scan_results() when is_scan_done() returns true.
    #[func]
    fn start_scan(&self, timeout_seconds: f32) -> bool {
        let timeout_ms = ((timeout_seconds.max(1.0)) * 1000.0) as i32;

        {
            let mut state = self.scan_state.lock().unwrap();
            if matches!(*state, ScanState::Scanning) {
                godot_print!("GdBLE: Scan already in progress");
                return false;
            }
            *state = ScanState::Scanning;
        }

        let adapter_arc = self.adapter.clone();
        let scan_state_arc = self.scan_state.clone();

        // NOTE: godot_print!/godot_error! are main-thread-only in gdext's
        // single_threaded binding — use println!/eprintln! in the thread.
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let adapter_guard = adapter_arc.lock().unwrap();
                let adapter = match adapter_guard.as_ref() {
                    Some(a) => a,
                    None => return Err("Not initialized — call initialize() first".to_string()),
                };

                println!("GdBLE: Scanning for {} ms…", timeout_ms);

                adapter
                    .scan_for(timeout_ms)
                    .map_err(|e| format!("scan_for failed: {}", e))?;

                let peripherals = adapter
                    .scan_get_results()
                    .map_err(|e| format!("scan_get_results failed: {}", e))?;

                println!("GdBLE: Raw scan found {} peripheral(s)", peripherals.len());
                Ok(peripherals)
                // adapter_guard drops here — before ScanState is set
            }));

            let new_state = match result {
                Ok(Ok(peripherals)) => ScanState::Done(peripherals),
                Ok(Err(msg)) => {
                    eprintln!("GdBLE scan error: {}", msg);
                    ScanState::Error(msg)
                }
                Err(_panic) => {
                    let msg = "scan thread panicked".to_string();
                    eprintln!("GdBLE: {}", msg);
                    ScanState::Error(msg)
                }
            };

            if let Ok(mut state) = scan_state_arc.lock() {
                *state = new_state;
            }
        });

        true
    }

    /// Returns true when a scan has finished and results are ready.
    #[func]
    fn is_scan_done(&self) -> bool {
        matches!(*self.scan_state.lock().unwrap(), ScanState::Done(_))
    }

    /// Returns true while a scan is running.
    #[func]
    fn is_scanning(&self) -> bool {
        matches!(*self.scan_state.lock().unwrap(), ScanState::Scanning)
    }

    /// Returns a non-empty error string if the last scan failed, empty string otherwise.
    /// Resets state to Idle so a new scan can start.
    #[func]
    fn take_scan_error(&self) -> GString {
        let mut state = self.scan_state.lock().unwrap();
        if let ScanState::Error(msg) = &*state {
            let out = GString::from(msg.as_str());
            *state = ScanState::Idle;
            out
        } else {
            GString::new()
        }
    }

    /// Collect scan results as BLEDevice objects. Resets state to Idle.
    /// MUST be called from the main thread — Gd<BLEDevice> creation requires it.
    #[func]
    fn take_scan_results(&self) -> Array<Gd<BLEDevice>> {
        let peripherals: Vec<Peripheral> = {
            let mut state = self.scan_state.lock().unwrap();
            if let ScanState::Done(p) = std::mem::replace(&mut *state, ScanState::Idle) {
                p
            } else {
                Vec::new()
            }
        };

        let mut devices = Array::new();
        for peripheral in peripherals {
            let device = Gd::from_init_fn(|base| {
                BLEDevice::from_peripheral(base, peripheral, self.runtime.clone())
            });
            devices.push(&device);
        }
        devices
    }

    /// Returns true if the adapter and runtime are ready.
    #[func]
    fn is_initialized(&self) -> bool {
        self.adapter.lock().unwrap().is_some() && self.runtime.lock().unwrap().is_some()
    }
}
