# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and run

```sh
cargo build
cargo run -- <device-mac-address>
cargo run -- --adapter hci1 <device-mac-address>
```

There are no tests.

## Architecture

A single-binary Rust async app (`src/main.rs`) that bridges iOS notifications to Linux desktop notifications via BLE. It operates in two roles simultaneously:

- **BLE Central (ANCS client)** — connects to the iOS device, subscribes to ANCS notifications, and forwards them to the desktop.
- **BLE Peripheral (HID server)** — advertises a dummy HID keyboard GATT service on the local adapter. This tricks iOS into treating the Linux machine as a keyboard, which triggers automatic background reconnection — iOS does not auto-reconnect to ANCS-only devices.

On startup, `main` registers the HID GATT application and starts advertising before entering the ANCS retry loop.

- **`serve_hid_gatt`** — builds a minimal HID-over-GATT service (`0x1812`) with the five mandatory characteristics (HID Information, Report Map, HID Control Point, Protocol Mode, Report) and a Report Reference descriptor, registers it via `adapter.serve_gatt_application()`, then starts BLE advertising with HID service UUID and keyboard appearance (`0x03C1`). Returns handles that must be kept alive for the application lifetime.
- **`AncsProcessor`** — holds the ANCS GATT control point characteristic and a `HashMap<String, String>` cache of app identifier → display name.

`AncsProcessor::main_loop` connects to the paired iOS device, discovers the ANCS GATT service and its three required characteristics:
1. **Notification Source** (notify/indicate) — signals new/modified notifications.
2. **Data Source** (notify/indicate) — carries notification content and app attribute responses.
3. **Control Point** (write) — sends commands to request attributes.

The core of `main_loop` is a `tokio::select!` over three async streams: notification source, data source, and adapter events (to detect device disconnection).

- `process_notification` — parses the incoming notification header (event ID, flags, UID), skips pre-existing/removed notifications, then writes a `GetNotificationAttributes` command to the control point requesting app identifier, title, subtitle, and message (each truncated to 64 bytes).
- `process_data` — dispatches on the first byte: `0x00` = notification attributes (builds and shows a `notify-rust` desktop notification; if the app name isn't cached, issues a `GetAppAttributes` command); `0x01` = app attributes (parses the display name and caches it).
- `write_control_point` — sends a buffer to the control point characteristic using write-without-response.

Key dependencies: `bluer` 0.17 (async BlueZ GATT client and server, BLE advertising, uses `UuidExt` for `Uuid::from_u16`), `ancs` 0.2 (ANCS protocol types and parsing), `notify-rust` 4.11 (freedesktop desktop notifications), `tokio` 1.x (async runtime, all features).
