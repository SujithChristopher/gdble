use godot::prelude::*;
use godot::classes::{RefCounted, IRefCounted};
use btleplug::api::{Peripheral as _, WriteType};
use btleplug::platform::Peripheral;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;
use uuid::Uuid;

#[derive(GodotClass)]
#[class(base=RefCounted)]
pub struct BLEDevice {
    base: Base<RefCounted>,
    peripheral: Arc<Mutex<Peripheral>>,
    runtime: Arc<Mutex<Option<Runtime>>>,
    name: GString,
    address: GString,
    is_connected: bool,
    /// Latest raw bytes received per characteristic UUID (populated by background task)
    notifications: Arc<Mutex<HashMap<Uuid, Vec<u8>>>>,
    /// Background task handle for the notification stream; aborted on unsubscribe/disconnect
    notification_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

#[godot_api]
impl IRefCounted for BLEDevice {
    fn init(base: Base<RefCounted>) -> Self {
        Self {
            base,
            peripheral: Arc::new(Mutex::new(unsafe { std::mem::zeroed() })),
            runtime: Arc::new(Mutex::new(None)),
            name: GString::from("Unknown"),
            address: GString::from(""),
            is_connected: false,
            notifications: Arc::new(Mutex::new(HashMap::new())),
            notification_task: Arc::new(Mutex::new(None)),
        }
    }
}

impl BLEDevice {
    pub fn from_peripheral(
        base: Base<RefCounted>,
        peripheral: Peripheral,
        runtime: Arc<Mutex<Option<Runtime>>>,
    ) -> Self {
        let runtime_guard = runtime.lock().unwrap();
        let mut name = GString::from("Unknown");
        let mut address = GString::from("");

        if let Some(rt) = runtime_guard.as_ref() {
            if let Ok(Some(props)) = rt.block_on(peripheral.properties()) {
                if let Some(local_name) = props.local_name {
                    name = GString::from(local_name.as_str());
                }
                address = GString::from(props.address.to_string().as_str());
            }
        }

        drop(runtime_guard);

        Self {
            base,
            peripheral: Arc::new(Mutex::new(peripheral)),
            runtime,
            name,
            address,
            is_connected: false,
            notifications: Arc::new(Mutex::new(HashMap::new())),
            notification_task: Arc::new(Mutex::new(None)),
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

    /// Named ble_is_connected to avoid conflict with GDScript's built-in Object.is_connected().
    #[func]
    fn ble_is_connected(&self) -> bool {
        self.is_connected
    }

    // ── connection ────────────────────────────────────────────────────────────

    /// Connect to this device and automatically discover services.
    /// Blocking — call from a GDScript Thread.
    /// Named ble_connect to avoid conflict with GDScript's built-in Object.connect().
    #[func]
    fn ble_connect(&mut self) -> bool {
        let runtime_guard = self.runtime.lock().unwrap();
        let peripheral = self.peripheral.lock().unwrap();

        if let Some(rt) = runtime_guard.as_ref() {
            match rt.block_on(peripheral.connect()) {
                Ok(_) => {
                    godot_print!("GdBLE: Connected to '{}'", self.name);
                    self.is_connected = true;
                    if let Err(e) = rt.block_on(peripheral.discover_services()) {
                        godot_error!("GdBLE: Service discovery failed: {}", e);
                    }
                    true
                }
                Err(e) => {
                    godot_error!("GdBLE: Connection to '{}' failed: {}", self.name, e);
                    false
                }
            }
        } else {
            godot_error!("GdBLE: Runtime not available");
            false
        }
    }

    /// Disconnect and abort any active notification stream.
    /// Named ble_disconnect to avoid conflict with GDScript's built-in Object.disconnect().
    #[func]
    fn ble_disconnect(&mut self) -> bool {
        self._abort_notification_task();

        let runtime_guard = self.runtime.lock().unwrap();
        let peripheral = self.peripheral.lock().unwrap();

        if let Some(rt) = runtime_guard.as_ref() {
            match rt.block_on(peripheral.disconnect()) {
                Ok(_) => {
                    godot_print!("GdBLE: Disconnected from '{}'", self.name);
                    self.is_connected = false;
                    true
                }
                Err(e) => {
                    godot_error!("GdBLE: Disconnect failed: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }

    // ── notifications ─────────────────────────────────────────────────────────

    /// Subscribe to a characteristic and start a background listener.
    /// New values are stored internally; retrieve them with poll_notification().
    /// The service_uuid parameter is accepted for API consistency but ignored —
    /// btleplug locates characteristics by UUID alone after discover_services().
    #[func]
    fn subscribe(&mut self, _service_uuid: GString, char_uuid: GString) -> bool {
        if !self.is_connected {
            godot_error!("GdBLE: Cannot subscribe — device not connected");
            return false;
        }

        let char_uuid_parsed = match Uuid::parse_str(&char_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid characteristic UUID: {}", e); return false; }
        };

        // Clone peripheral (btleplug Peripheral is Clone — cheap Arc clone internally)
        let peripheral_clone = self.peripheral.lock().unwrap().clone();

        // Find the characteristic (synchronously — characteristics already discovered)
        let char_ref = match peripheral_clone
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == char_uuid_parsed)
        {
            Some(c) => c,
            None => {
                godot_error!("GdBLE: Characteristic {} not found (was discover_services called?)", char_uuid);
                return false;
            }
        };

        let runtime_guard = self.runtime.lock().unwrap();
        let rt = match runtime_guard.as_ref() {
            Some(rt) => rt,
            None => { godot_error!("GdBLE: Runtime not available"); return false; }
        };

        // Subscribe (blocking)
        let peripheral_for_sub = peripheral_clone.clone();
        let char_for_sub = char_ref.clone();
        if let Err(e) = rt.block_on(async move { peripheral_for_sub.subscribe(&char_for_sub).await }) {
            godot_error!("GdBLE: Subscribe failed: {}", e);
            return false;
        }

        // Spawn a background async task to drain the notification stream
        let notifications_store = self.notifications.clone();
        let peripheral_for_task = peripheral_clone.clone();
        let handle = rt.handle().clone();

        let task = handle.spawn(async move {
            match peripheral_for_task.notifications().await {
                Ok(mut stream) => {
                    while let Some(notif) = stream.next().await {
                        notifications_store
                            .lock()
                            .unwrap()
                            .insert(notif.uuid, notif.value);
                    }
                    godot_print!("GdBLE: Notification stream closed");
                }
                Err(e) => godot_error!("GdBLE: Could not open notification stream: {}", e),
            }
        });

        *self.notification_task.lock().unwrap() = Some(task);
        godot_print!("GdBLE: Subscribed to {}", char_uuid);
        true
    }

    /// Unsubscribe from a characteristic and stop the background listener.
    #[func]
    fn unsubscribe(&mut self, _service_uuid: GString, char_uuid: GString) -> bool {
        if !self.is_connected { return false; }

        let char_uuid_parsed = match Uuid::parse_str(&char_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid UUID: {}", e); return false; }
        };

        let peripheral_clone = self.peripheral.lock().unwrap().clone();
        let char_ref = match peripheral_clone
            .characteristics()
            .into_iter()
            .find(|c| c.uuid == char_uuid_parsed)
        {
            Some(c) => c,
            None => return false,
        };

        self._abort_notification_task();

        let runtime_guard = self.runtime.lock().unwrap();
        if let Some(rt) = runtime_guard.as_ref() {
            if let Err(e) = rt.block_on(async move { peripheral_clone.unsubscribe(&char_ref).await }) {
                godot_error!("GdBLE: Unsubscribe failed: {}", e);
                return false;
            }
            true
        } else {
            false
        }
    }

    /// Return the latest notification bytes for a characteristic UUID, then clear it.
    /// Returns an empty PackedByteArray if no new data has arrived since the last call.
    /// Call this every frame from _process() to receive streaming data.
    #[func]
    fn poll_notification(&self, char_uuid: GString) -> PackedByteArray {
        let mut result = PackedByteArray::new();

        let char_uuid_parsed = match Uuid::parse_str(&char_uuid.to_string()) {
            Ok(u) => u,
            Err(_) => return result,
        };

        if let Some(data) = self.notifications.lock().unwrap().remove(&char_uuid_parsed) {
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

        let service_uuid_parsed = match Uuid::parse_str(&service_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid service UUID: {}", e); return false; }
        };

        let char_uuid_parsed = match Uuid::parse_str(&characteristic_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid characteristic UUID: {}", e); return false; }
        };

        let runtime_guard = self.runtime.lock().unwrap();
        let peripheral = self.peripheral.lock().unwrap();

        if let Some(rt) = runtime_guard.as_ref() {
            let result = rt.block_on(async {
                let chars = peripheral.characteristics();
                let char_ref = chars
                    .iter()
                    .find(|c| c.service_uuid == service_uuid_parsed && c.uuid == char_uuid_parsed);

                if let Some(ch) = char_ref {
                    peripheral.write(ch, &data.to_vec(), WriteType::WithoutResponse).await
                } else {
                    Err(btleplug::Error::NotSupported("Characteristic not found".to_string()))
                }
            });

            match result {
                Ok(_) => true,
                Err(e) => { godot_error!("GdBLE: Write failed: {}", e); false }
            }
        } else {
            false
        }
    }

    #[func]
    fn read(&self, service_uuid: GString, characteristic_uuid: GString) -> PackedByteArray {
        let mut result = PackedByteArray::new();

        if !self.is_connected {
            godot_error!("GdBLE: Device not connected");
            return result;
        }

        let service_uuid_parsed = match Uuid::parse_str(&service_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid service UUID: {}", e); return result; }
        };

        let char_uuid_parsed = match Uuid::parse_str(&characteristic_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid characteristic UUID: {}", e); return result; }
        };

        let runtime_guard = self.runtime.lock().unwrap();
        let peripheral = self.peripheral.lock().unwrap();

        if let Some(rt) = runtime_guard.as_ref() {
            let read_result = rt.block_on(async {
                let chars = peripheral.characteristics();
                let char_ref = chars
                    .iter()
                    .find(|c| c.service_uuid == service_uuid_parsed && c.uuid == char_uuid_parsed);

                if let Some(ch) = char_ref {
                    peripheral.read(ch).await
                } else {
                    Err(btleplug::Error::NotSupported("Characteristic not found".to_string()))
                }
            });

            match read_result {
                Ok(data) => { for byte in data { result.push(byte); } }
                Err(e) => { godot_error!("GdBLE: Read failed: {}", e); }
            }
        }
        result
    }

    // ── service discovery ─────────────────────────────────────────────────────

    #[func]
    fn get_services(&self) -> PackedStringArray {
        let mut services = PackedStringArray::new();
        if !self.is_connected { return services; }
        let peripheral = self.peripheral.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        for ch in peripheral.characteristics() {
            if seen.insert(ch.service_uuid) {
                services.push(ch.service_uuid.to_string().as_str());
            }
        }
        services
    }

    #[func]
    fn get_characteristics(&self, service_uuid: GString) -> PackedStringArray {
        let mut chars = PackedStringArray::new();
        if !self.is_connected { return chars; }
        let svc_uuid = match Uuid::parse_str(&service_uuid.to_string()) {
            Ok(u) => u,
            Err(e) => { godot_error!("GdBLE: Invalid service UUID: {}", e); return chars; }
        };
        let peripheral = self.peripheral.lock().unwrap();
        for ch in peripheral.characteristics() {
            if ch.service_uuid == svc_uuid {
                chars.push(ch.uuid.to_string().as_str());
            }
        }
        chars
    }

    // ── internal helpers ──────────────────────────────────────────────────────

    fn _abort_notification_task(&self) {
        if let Some(task) = self.notification_task.lock().unwrap().take() {
            task.abort();
        }
    }
}
