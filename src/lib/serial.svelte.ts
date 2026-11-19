// App-wide serial terminal session. Lives outside the panel component so the
// log and connection survive switching between compact and KVM views (each
// view mounts its own SerialPanel).

import { commands, events } from "./ipc";
import { TerminalLog, type LineEnding } from "./serialTerminal";
import type { SerialConfig } from "./types";

export interface SerialSettings extends SerialConfig {
  port: string;
  lineEnding: LineEnding;
  timestamps: boolean;
  hex: boolean;
  hexSend: boolean;
  echo: boolean;
}

const SETTINGS_KEY = "serialSettings";
const defaults: SerialSettings = {
  port: "",
  baudRate: 115200,
  dataBits: 8,
  parity: "none",
  stopBits: 1,
  flowControl: "none",
  lineEnding: "crlf",
  timestamps: false,
  hex: false,
  hexSend: false,
  echo: true,
};

function loadSettings(): SerialSettings {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(SETTINGS_KEY) ?? "{}") };
  } catch {
    return { ...defaults };
  }
}

export const log = new TerminalLog(5000);

export const serial = $state({
  open: false,
  port: null as string | null,
  error: null as string | null,
  /** Bumped on every log change so views re-render (the log itself isn't reactive). */
  version: 0,
  settings: loadSettings(),
  history: [] as string[],
});

export function saveSettings() {
  localStorage.setItem(SETTINGS_KEY, JSON.stringify(serial.settings));
}

export function touchLog() {
  serial.version++;
}

let started = false;

/** Subscribe to backend serial events once for the app's lifetime. */
export function initSerial() {
  if (started) return;
  started = true;
  events.onSerialData((bytes) => {
    log.append(bytes);
    touchLog();
  });
  events.onSerialState((s) => {
    serial.open = s.open;
    serial.port = s.port;
    if (s.error) serial.error = s.error;
  });
  commands
    .serialState()
    .then((s) => {
      serial.open = s.open;
      serial.port = s.port;
    })
    .catch(() => {
      /* outside Tauri */
    });
}
