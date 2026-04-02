use godot::prelude::*;
use godot::classes::{RefCounted, IRefCounted};
use btleplug::api::{Central, Manager as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral};
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
    runtime: Arc<Mutex<Option<Runtime>>>,
    manager: Arc<Mutex<Option<Manager>>>,
    scan_state: Arc<Mutex<ScanState>>,
}

#[godot_api]
impl IRefCounted for GdBLE {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            runtime: Arc::new(Mutex::new(None)),
            manager: Arc::new(Mutex::new(None)),
            scan_state: Arc::new(Mutex::new(ScanState::Idle)),
        }
    }
}

#[godot_api]
impl GdBLE {
    /// Initialize the Bluetooth LE manager.
    /// Returns true if successful.
    #[func]
    fn initialize(&mut self) -> bool {
        let rt = Runtime::new();
        match rt {
            Ok(runtime) => {
                let manager_result = runtime.block_on(async {
                    Manager::new().await
                });
                match manager_result {
                    Ok(manager) => {
                        *self.runtime.lock().unwrap() = Some(runtime);
                        *self.manager.lock().unwrap() = Some(manager);
                        godot_print!("GdBLE: Bluetooth LE initialized successfully");
                        true
                    }
                    Err(e) => {
                        godot_error!("GdBLE: Failed to create BLE manager: {}", e);
                        false
                    }
                }
            }
            Err(e) => {
                godot_error!("GdBLE: Failed to create runtime: {}", e);
                false
            }
        }
    }

    /// Start a non-blocking background scan. Safe to call from the main thread.
    /// Returns false if a scan is already running.
    /// Poll is_scan_done() or is_scan_error() each frame.
    /// Call take_scan_results() when is_scan_done() returns true.
    #[func]
    fn start_scan(&self, timeout_seconds: f32) -> bool {
        let timeout = if timeout_seconds <= 0.0 { 5.0 } else { timeout_seconds };

        {
            let mut state = self.scan_state.lock().unwrap();
            if matches!(*state, ScanState::Scanning) {
                godot_print!("GdBLE: Scan already in progress");
                return false;
            }
            *state = ScanState::Scanning;
        }

        let runtime_arc = self.runtime.clone();
        let manager_arc = self.manager.clone();
        let scan_state_arc = self.scan_state.clone();

        // NOTE: godot_print!/godot_error! are main-thread-only in gdext's single_threaded
        // binding. Use println!/eprintln! inside the thread instead.
        std::thread::spawn(move || {
            // Wrap the entire thread body to catch panics and surface them to GDScript.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let peripherals: Vec<Peripheral> = {
                    let runtime_guard = match runtime_arc.lock() {
                        Ok(g) => g,
                        Err(e) => {
                            return Err(format!("runtime mutex poisoned: {}", e));
                        }
                    };
                    let manager_guard = match manager_arc.lock() {
                        Ok(g) => g,
                        Err(e) => {
                            return Err(format!("manager mutex poisoned: {}", e));
                        }
                    };

                    if let (Some(runtime), Some(manager)) =
                        (runtime_guard.as_ref(), manager_guard.as_ref())
                    {
                        let scan_result = runtime.block_on(async {
                            let adapters = manager.adapters().await
                                .map_err(|e| format!("adapters() failed: {}", e))?;

                            if adapters.is_empty() {
                                return Err("No Bluetooth adapters found".to_string());
                            }

                            let central = &adapters[0];
                            println!("GdBLE: Scanning for {} seconds…", timeout);

                            central.start_scan(ScanFilter::default()).await
                                .map_err(|e| format!("start_scan failed: {}", e))?;

                            tokio::time::sleep(
                                tokio::time::Duration::from_secs_f32(timeout)
                            ).await;

                            central.stop_scan().await
                                .map_err(|e| format!("stop_scan failed: {}", e))?;

                            let peripherals = central.peripherals().await
                                .map_err(|e| format!("peripherals() failed: {}", e))?;

                            println!("GdBLE: Raw scan found {} peripheral(s)", peripherals.len());
                            Ok(peripherals)
                        });

                        match scan_result {
                            Ok(p) => p,
                            Err(e) => return Err(e),
                        }
                    } else {
                        return Err("Not initialized — call initialize() first".to_string());
                    }
                    // runtime_guard and manager_guard are dropped here, before ScanState is set
                };

                Ok(peripherals)
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

    /// Returns true when a scan has finished successfully and results are ready.
    #[func]
    fn is_scan_done(&self) -> bool {
        matches!(*self.scan_state.lock().unwrap(), ScanState::Done(_))
    }

    /// Returns true if the scan is currently running.
    #[func]
    fn is_scanning(&self) -> bool {
        matches!(*self.scan_state.lock().unwrap(), ScanState::Scanning)
    }

    /// Returns a non-empty error string if the last scan failed, empty string otherwise.
    /// Resets the error state to Idle so a new scan can start.
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

    /// Collect completed scan results as BLEDevice objects and reset state to Idle.
    /// MUST be called from the main thread — Gd<BLEDevice> creation requires it.
    #[func]
    fn take_scan_results(&self) -> Array<Gd<BLEDevice>> {
        // Extract peripherals and release the scan_state lock before touching runtime.
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

    /// Returns true if GdBLE is initialized and ready.
    #[func]
    fn is_initialized(&self) -> bool {
        let runtime_guard = self.runtime.lock().unwrap();
        let manager_guard = self.manager.lock().unwrap();
        runtime_guard.is_some() && manager_guard.is_some()
    }
}
