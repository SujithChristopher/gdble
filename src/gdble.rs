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
    /// Poll is_scan_done() each frame; call take_scan_results() when it returns true.
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

        std::thread::spawn(move || {
            // Hold BLE locks only for the duration of the scan, then release
            // before writing ScanState::Done so take_scan_results() can lock runtime.
            let peripherals: Vec<Peripheral> = {
                let runtime_guard = runtime_arc.lock().unwrap();
                let manager_guard = manager_arc.lock().unwrap();

                if let (Some(runtime), Some(manager)) =
                    (runtime_guard.as_ref(), manager_guard.as_ref())
                {
                    let result = runtime.block_on(async {
                        let adapters = manager.adapters().await?;
                        if adapters.is_empty() {
                            godot_error!("GdBLE: No Bluetooth adapters found");
                            return Ok::<Vec<Peripheral>, btleplug::Error>(Vec::new());
                        }
                        let central = &adapters[0];
                        godot_print!("GdBLE: Scanning for {} seconds…", timeout);
                        central.start_scan(ScanFilter::default()).await?;
                        tokio::time::sleep(
                            tokio::time::Duration::from_secs_f32(timeout)
                        ).await;
                        central.stop_scan().await?;
                        Ok(central.peripherals().await?)
                    });

                    match result {
                        Ok(p) => p,
                        Err(e) => {
                            godot_error!("GdBLE: Scan failed: {}", e);
                            Vec::new()
                        }
                    }
                } else {
                    godot_error!("GdBLE: Not initialized. Call initialize() first.");
                    Vec::new()
                }
                // runtime_guard and manager_guard are dropped here, before ScanState is set
            };

            *scan_state_arc.lock().unwrap() = ScanState::Done(peripherals);
        });

        true
    }

    /// Returns true when a scan has finished and results are ready.
    #[func]
    fn is_scan_done(&self) -> bool {
        matches!(*self.scan_state.lock().unwrap(), ScanState::Done(_))
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
