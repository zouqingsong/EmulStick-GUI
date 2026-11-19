// Mirrors the Rust IPC DTOs in src-tauri/src/ipc/mod.rs. Keep in sync.

export interface DiscoveredDevice {
  /** Opaque, machine-local Peripheral::id() — NOT a portable MAC address. */
  id: string;
  name: string | null;
  rssi: number | null;
}

// Auto-reconnect is frontend-driven (see App.svelte's `reconnecting` flag); the
// backend never emits a "Reconnecting" state, so it isn't part of this union.
export type ConnectionState =
  | "Scanning"
  | "Connecting"
  | "Connected"
  | "Disconnected";

export interface DeviceInfo {
  firmware: string | null;
  model: string | null;
  manufacturer: string | null;
  systemId: string | null;
}

export interface PassthroughFlags {
  keyboard: boolean;
  mouse: boolean;
}

export interface LedReport {
  num: boolean;
  caps: boolean;
  scroll: boolean;
}

export interface ErrorEvent {
  code: string;
  message: string;
}

export interface SerialPortInfo {
  name: string;
  description: string | null;
}

export interface SerialConfig {
  baudRate: number;
  dataBits: 5 | 6 | 7 | 8;
  parity: "none" | "odd" | "even";
  stopBits: 1 | 2;
  flowControl: "none" | "software" | "hardware";
}

export interface SerialState {
  open: boolean;
  port: string | null;
  /** Set when the port closed unexpectedly (e.g. adapter unplugged). */
  error: string | null;
}
