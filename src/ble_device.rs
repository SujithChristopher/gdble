use godot::prelude::*;
use godot::classes::{RefCounted, IRefCounted};
use simplersble::{Peripheral, ValueChangedEvent};
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct BLEDevice {
    base: Base<RefCounted>,
    peripheral: Option<Arc<Peripheral>>,
    runtime: Arc<Mutex<Option<Runtime>>>,
    name: GString,
    address: GString,
    is_connected: bool,
    /// Latest notification bytes keyed by lowercase characteristic UUID.
    notifications: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    /// Background tasks draining notification streams; aborted on unsubscribe/disconnect.
    notification_tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

#[godot_api]
impl IRefCounted for BLEDevice {
    fn init(base: Base<RefCounted>) -> Self {
        // This default init is required by gdext but should not be used directly;
        // use from_peripheral() instead.
        Self {
            base,
            peripheral: None,
            runtime: Arc::new(Mutex::new(None)),
            name: GString::from("Unknown"),
            address: GString::from(""),
            is_connected: false,
            notifications: Arc::new(Mutex::new(HashMap::new())),
            notification_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl BLEDevice {
    /// Construct a BLEDevice from a scanned peripheral.
    /// Must be called on the main thread.
    pub fn from_peripheral(
        base: Base<RefCounted>,
        peripheral: Peripheral,
        runtime: Arc<Mutex<Option<Runtime>>>,
    ) -> Self {
        let name = peripheral
            .identifier()
            .unwrap_or_else(|_| "Unknown".to_string());
        let address = peripheral
            .address()
            .unwrap_or_else(|_| "".to_string());

        Self {
            base,
            peripheral: Some(Arc::new(peripheral)),
            runtime,
            name: GString::from(name.as_str()),
            address: GString::from(address.as_str()),
            is_connected: false,
            notifications: Arc::new(Mutex::new(HashMap::new())),
            notification_tasks: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[godot_api]
impl BLEDevice {
    // ── identity ──────────────────────────────────────────────────────────────

    #[func]
    fn get_name(&self) -> GString {
        self.name.clone()
    }

    #[func]
    fn get_address(&self) -> GString {
        self.address.clone()
    }

    /// Named ble_is_connected to avoid conflict with GDScript's built-in is_connected().
    #[func]
    fn ble_is_connected(&self) -> bool {
        self.is_connected
    }

    // ── connection ────────────────────────────────────────────────────────────

    /// Connect to this device and discover services.
    /// Named ble_connect to avoid conflict with GDScript's built-in connect().
    #[func]
    fn ble_connect(&mut self) -> bool {
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return false;
        };

        match peripheral.connect() {
            Ok(_) => {
                self.is_connected = true;
                godot_print!("GdBLE: Connected to '{}'", self.name);
                true
            }
            Err(e) => {
                godot_error!("GdBLE: Connection failed: {}", e);
                false
            }
        }
    }

    /// Disconnect and abort all active notification streams.
    /// Named ble_disconnect to avoid conflict with GDScript's built-in disconnect().
    #[func]
    fn ble_disconnect(&mut self) -> bool {
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return false;
        };

        self._abort_all_tasks();
        match peripheral.disconnect() {
            Ok(_) => {
                self.is_connected = false;
                godot_print!("GdBLE: Disconnected from '{}'", self.name);
                true
            }
            Err(e) => {
                godot_error!("GdBLE: Disconnect failed: {}", e);
                false
            }
        }
    }

    // ── notifications ─────────────────────────────────────────────────────────

    /// Subscribe to a characteristic and start a background listener.
    /// Retrieve data with poll_notification() each frame.
    #[func]
    fn subscribe(&mut self, service_uuid: GString, char_uuid: GString) -> bool {
        if !self.is_connected {
            godot_error!("GdBLE: Cannot subscribe — not connected");
            return false;
        }

        let svc = service_uuid.to_string();
        let chr = char_uuid.to_string();
        let key = chr.to_lowercase();
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return false;
        };

        // Ask the C++ library to start sending notifications and get the event stream.
        let stream = match peripheral.notify(&svc, &chr) {
            Ok(s) => s,
            Err(e) => {
                godot_error!("GdBLE: notify() failed: {}", e);
                return false;
            }
        };

        let notifications = self.notifications.clone();

        // Spawn a tokio task to drain the stream into the notifications map.
        // The task runs on the shared runtime so it stays alive as long as GdBLE does.
        let runtime_guard = self.runtime.lock().unwrap();
        let Some(rt) = runtime_guard.as_ref() else {
            godot_error!("GdBLE: No runtime — was initialize() called?");
            return false;
        };

        let handle = rt.handle().clone();
        drop(runtime_guard);

        let task = handle.spawn(async move {
            let mut stream = std::pin::pin!(stream);
            while let Some(event) = stream.next().await {
                if let Ok(ValueChangedEvent::ValueUpdated(data)) = event {
                    notifications.lock().unwrap().insert(key.clone(), data);
                }
            }
        });

        self.notification_tasks.lock().unwrap().push(task);
        godot_print!("GdBLE: Subscribed to {}", char_uuid);
        true
    }

    /// Unsubscribe from a characteristic.
    #[func]
    fn unsubscribe(&mut self, service_uuid: GString, char_uuid: GString) -> bool {
        let svc = service_uuid.to_string();
        let chr = char_uuid.to_string();
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return false;
        };

        match peripheral.unsubscribe(&svc, &chr) {
            Ok(_) => true,
            Err(e) => {
                godot_error!("GdBLE: unsubscribe failed: {}", e);
                false
            }
        }
    }

    /// Return the latest notification bytes for a characteristic UUID, then clear it.
    /// Returns an empty PackedByteArray if no new data has arrived since last call.
    /// Call every frame from _process() to receive streaming data.
    #[func]
    fn poll_notification(&self, char_uuid: GString) -> PackedByteArray {
        let key = char_uuid.to_string().to_lowercase();
        let mut result = PackedByteArray::new();
        if let Some(data) = self.notifications.lock().unwrap().remove(&key) {
            for byte in data {
                result.push(byte);
            }
        }
        result
    }

    // ── read / write ──────────────────────────────────────────────────────────

    #[func]
    fn write(&self, service_uuid: GString, characteristic_uuid: GString, data: PackedByteArray) -> bool {
        if !self.is_connected {
            godot_error!("GdBLE: Device not connected");
            return false;
        }
        let svc = service_uuid.to_string();
        let chr = characteristic_uuid.to_string();
        let bytes = data.to_vec();
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return false;
        };

        match peripheral.write_command(&svc, &chr, &bytes) {
            Ok(_) => true,
            Err(e) => {
                godot_error!("GdBLE: write_command failed: {}", e);
                false
            }
        }
    }

    #[func]
    fn read(&self, service_uuid: GString, characteristic_uuid: GString) -> PackedByteArray {
        let mut result = PackedByteArray::new();
        if !self.is_connected {
            godot_error!("GdBLE: Device not connected");
            return result;
        }
        let svc = service_uuid.to_string();
        let chr = characteristic_uuid.to_string();
        let Some(peripheral) = self.peripheral() else {
            godot_error!("GdBLE: Device handle is not initialized");
            return result;
        };

        match peripheral.read(&svc, &chr) {
            Ok(data) => {
                for byte in data {
                    result.push(byte);
                }
            }
            Err(e) => godot_error!("GdBLE: read failed: {}", e),
        }
        result
    }

    // ── internal helpers ──────────────────────────────────────────────────────

    fn peripheral(&self) -> Option<&Peripheral> {
        self.peripheral.as_deref()
    }

    fn _abort_all_tasks(&self) {
        let mut tasks = self.notification_tasks.lock().unwrap();
        for task in tasks.drain(..) {
            task.abort();
        }
    }
}
