# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

GdBLE is a Rust-based Bluetooth LE (BLE) GDExtension for Godot 4.2+. It provides a simple, Pythonic API for scanning, connecting to, and communicating with Bluetooth LE devices. Built using:
- **gdext**: Rust bindings for Godot 4
- **btleplug**: Cross-platform Bluetooth LE library
- **tokio**: Async runtime for BLE operations

## Build Commands

### Windows
```bash
build.bat
```
Builds for x86_64-pc-windows-msvc target and copies `gdble.dll` to `addons/gdble/bin/windows-x86_64/`

### Linux/macOS
```bash
./build.sh
```
Auto-detects platform (Linux/macOS) and architecture (x86_64/arm64), builds the appropriate target, and copies the library to the correct `addons/gdble/bin/{platform}/` directory.

### Manual Build
```bash
cargo build --release --target <target-triple>
```

## Architecture

### GDExtension Structure

The project follows the standard GDExtension addon pattern:
- **Extension entry point**: `src/lib.rs` defines `GdBLEExtension` with the `#[gdextension]` macro
- **Library type**: `cdylib` (dynamic library) specified in `Cargo.toml`
- **Entry symbol**: `gdext_rust_init` - called by Godot when loading the extension
- **Addon location**: `addons/gdble/` contains the `.gdextension` file and platform-specific binaries

### Core Classes

**GdBLE** (`src/gdble.rs`):
- Main entry point for BLE operations
- Manages the tokio runtime and btleplug Manager
- Methods: `initialize()`, `scan()`, `is_initialized()`
- Uses `Arc<Mutex<>>` for thread-safe sharing of runtime and manager

**BLEDevice** (`src/ble_device.rs`):
- Represents an individual BLE peripheral
- Wraps btleplug's `Peripheral` type
- Methods: `connect()`, `disconnect()`, `read()`, `write()`, `get_services()`, `get_characteristics()`
- Shares the tokio runtime from GdBLE for async operations

### Async/Runtime Handling

The extension uses a **blocking pattern** over async:
1. GdBLE creates a tokio Runtime on `initialize()`
2. Runtime is stored in `Arc<Mutex<Option<Runtime>>>` and shared with BLEDevice instances
3. All async BLE operations use `runtime.block_on()` to convert async to sync
4. This pattern allows GDScript to call synchronous methods that internally handle async BLE operations

### Library Paths

The `.gdextension` file uses platform-specific paths pointing to `res://addons/gdble/bin/{platform}/`:
- Windows: `windows-x86_64/gdble.dll`, `windows-arm64/gdble.dll`
- Linux: `linux-x86_64/libgdble.so`, `linux-arm64/libgdble.so`
- macOS: `macos-x86_64/libgdble.dylib`, `macos-arm64/libgdble.dylib`

Both debug and release builds use the same binary paths (no separate debug builds deployed).

## Development Notes

### Adding New BLE Features

When adding new BLE functionality:
1. Add async btleplug calls in the appropriate class method
2. Use `runtime.block_on()` to execute the async operation synchronously
3. Handle errors with `godot_error!()` macro for logging to Godot console
4. Return GDScript-friendly types (bool, GString, PackedByteArray, etc.)

### GDScript API Design

The API is designed to be Pythonic and simple:
- Methods return simple success/failure booleans where appropriate
- UUIDs are passed as GString, converted to `Uuid` type internally
- Binary data uses `PackedByteArray` for GDScript compatibility
- Device discovery returns `Array<Gd<BLEDevice>>`

### Cross-Platform Considerations

- btleplug handles platform-specific Bluetooth APIs (Windows WinRT, Linux BlueZ, macOS CoreBluetooth)
- Build scripts must target specific platforms and copy to correct bin folders
- The `.gdextension` file maps each platform/arch combination to its binary location

## Fantety GDBLE (Alternative Source)

`fantety_gdble/` is a git submodule of https://github.com/Fantety/GDBLE.git — an alternative BLE implementation.
- Has `.gdignore` to prevent Godot from scanning it (would cause duplicate `.gdextension` conflicts)
- Excluded from Android export via `exclude_filter="fantety_gdble/*,gdble_src/*"` in `export_presets.cfg`
- Cross-compilation config in `fantety_gdble/.cargo/config.toml` uses NDK 29.0.13846066 at `C:\Users\Pintu\AppData\Local\Android\Sdk\ndk\29.0.13846066`
- Build for Android: `cd fantety_gdble && cargo build --release --target aarch64-linux-android`

## gdext Version Compatibility

- **Godot 4.5**: use `godot = "0.4.5"` (gdext 0.4.x)
- **Godot 4.6**: use `godot = { version = "0.5.0", features = ["api-4-6"] }` (gdext 0.5.x)
- Version mismatch loads on Windows with "safeguards balanced" warning but **silently fails on Android**
- gdext 0.5.0 breaking change: `dict.set()` requires `&` for `Array`, `Dictionary`, `GString`, `PackedByteArray`, `Variant` args
