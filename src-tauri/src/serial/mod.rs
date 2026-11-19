//! Serial (UART / RS-232) terminal transport.
//!
//! Independent of the BLE data plane: the operator plugs a USB-serial adapter
//! wired to the target's debug UART into *this* machine, and the UI shows its
//! log and sends lines back. A dedicated reader thread batches incoming bytes
//! into `serial_data` events (so a chatty console doesn't flood the IPC bridge
//! with one event per byte); writes go straight to a cloned port handle.

use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serialport::{DataBits, FlowControl, Parity, SerialPort, SerialPortType, StopBits};
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub const SERIAL_DATA: &str = "serial_data";
pub const SERIAL_STATE: &str = "serial_state";

/// Read poll timeout. Also the worst-case latency before a pending batch flushes.
const READ_TIMEOUT: Duration = Duration::from_millis(20);
/// Flush a batch at least this often while data keeps streaming in.
const BATCH_INTERVAL: Duration = Duration::from_millis(30);
/// …or as soon as it grows this large.
const BATCH_MAX: usize = 16 * 1024;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortInfo {
    pub name: String,
    /// Human-friendly description (USB product/manufacturer) when known.
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    /// "none" | "odd" | "even"
    pub parity: String,
    pub stop_bits: u8,
    /// "none" | "software" | "hardware"
    pub flow_control: String,
}

/// Payload of the `serial_state` event.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialState {
    pub open: bool,
    pub port: Option<String>,
    /// Set when the port closed unexpectedly (e.g. the adapter was unplugged).
    pub error: Option<String>,
}

struct Session {
    port_name: String,
    writer: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

/// Owns the (single) open serial session. Managed as Tauri state.
#[derive(Default)]
pub struct SerialManager {
    session: Mutex<Option<Session>>,
}

impl SerialManager {
    pub fn open<R: Runtime>(&self, app: &AppHandle<R>, port_name: &str, config: &SerialConfig) -> Result<(), String> {
        let settings = PortSettings::try_from(config)?;
        // Re-opening (e.g. a baud change) replaces any existing session.
        self.close(app);

        let port = serialport::new(port_name, config.baud_rate)
            .data_bits(settings.data_bits)
            .parity(settings.parity)
            .stop_bits(settings.stop_bits)
            .flow_control(settings.flow_control)
            .timeout(READ_TIMEOUT)
            .open()
            .map_err(|e| format!("Could not open {port_name}: {e}"))?;
        self.start(app, port_name, port)?;
        tracing::info!(port = port_name, baud = config.baud_rate, "serial port opened");
        Ok(())
    }

    /// Start a session on an already-opened port (spawns the reader thread).
    fn start<R: Runtime>(&self, app: &AppHandle<R>, port_name: &str, port: Box<dyn SerialPort>) -> Result<(), String> {
        self.close(app);
        let mut writer = port
            .try_clone()
            .map_err(|e| format!("Could not open {port_name}: {e}"))?;
        // The clone keeps its own timeout, so writes get a longer one than reads.
        let _ = writer.set_timeout(WRITE_TIMEOUT);

        let stop = Arc::new(AtomicBool::new(false));
        let reader = {
            let stop = stop.clone();
            let app = app.clone();
            let name = port_name.to_string();
            std::thread::Builder::new()
                .name("serial-reader".into())
                .spawn(move || reader_loop(app, port, name, stop))
                .map_err(|e| format!("Could not start serial reader: {e}"))?
        };

        *self.session.lock().unwrap() = Some(Session {
            port_name: port_name.to_string(),
            writer,
            stop,
            reader: Some(reader),
        });
        emit_state(app, true, Some(port_name.to_string()), None);
        Ok(())
    }

    /// Close the session if one is open. Idempotent.
    pub fn close<R: Runtime>(&self, app: &AppHandle<R>) {
        let session = self.session.lock().unwrap().take();
        if let Some(mut s) = session {
            s.stop.store(true, Ordering::SeqCst);
            // The reader wakes within READ_TIMEOUT; joining guarantees the port
            // is fully released before a re-open of the same device.
            if let Some(h) = s.reader.take() {
                let _ = h.join();
            }
            tracing::info!(port = s.port_name, "serial port closed");
            emit_state(app, false, None, None);
        }
    }

    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut guard = self.session.lock().unwrap();
        let s = guard.as_mut().ok_or("Serial port is not open")?;
        // No flush(): it is tcdrain(), which has no timeout and would block
        // forever (holding the lock) if hardware flow control stalls output.
        // write_all already honours WRITE_TIMEOUT per chunk.
        s.writer
            .write_all(data)
            .map_err(|e| format!("Serial write failed: {e}"))
    }

    pub fn set_signals(&self, dtr: Option<bool>, rts: Option<bool>) -> Result<(), String> {
        let mut guard = self.session.lock().unwrap();
        let s = guard.as_mut().ok_or("Serial port is not open")?;
        if let Some(v) = dtr {
            s.writer
                .write_data_terminal_ready(v)
                .map_err(|e| format!("Could not set DTR: {e}"))?;
        }
        if let Some(v) = rts {
            s.writer
                .write_request_to_send(v)
                .map_err(|e| format!("Could not set RTS: {e}"))?;
        }
        Ok(())
    }

    pub fn state(&self) -> SerialState {
        let guard = self.session.lock().unwrap();
        SerialState {
            open: guard.is_some(),
            port: guard.as_ref().map(|s| s.port_name.clone()),
            error: None,
        }
    }

    /// Drop the session after its reader hit a fatal error — but only if it's
    /// still the same session (a user re-open may have replaced it meanwhile).
    fn drop_failed(&self, stop: &Arc<AtomicBool>) -> bool {
        let mut guard = self.session.lock().unwrap();
        if guard.as_ref().is_some_and(|s| Arc::ptr_eq(&s.stop, stop)) {
            *guard = None;
            return true;
        }
        false
    }
}

fn reader_loop<R: Runtime>(app: AppHandle<R>, mut port: Box<dyn SerialPort>, name: String, stop: Arc<AtomicBool>) {
    let mut chunk = [0u8; 4096];
    let mut batch: Vec<u8> = Vec::with_capacity(BATCH_MAX);
    let mut batch_started = Instant::now();
    let flush = |batch: &mut Vec<u8>| {
        if !batch.is_empty() {
            let _ = app.emit(SERIAL_DATA, std::mem::take(batch));
        }
    };

    while !stop.load(Ordering::SeqCst) {
        match port.read(&mut chunk) {
            Ok(0) => flush(&mut batch),
            Ok(n) => {
                if batch.is_empty() {
                    batch_started = Instant::now();
                }
                batch.extend_from_slice(&chunk[..n]);
                if batch.len() >= BATCH_MAX || batch_started.elapsed() >= BATCH_INTERVAL {
                    flush(&mut batch);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::Interrupted | ErrorKind::WouldBlock) => {
                flush(&mut batch);
            }
            Err(e) => {
                flush(&mut batch);
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                tracing::warn!(port = name, ?e, "serial port read failed; closing");
                if app.state::<SerialManager>().drop_failed(&stop) {
                    emit_state(&app, false, None, Some(format!("{name} closed: {e}")));
                }
                return;
            }
        }
    }
    flush(&mut batch);
}

fn emit_state<R: Runtime>(app: &AppHandle<R>, open: bool, port: Option<String>, error: Option<String>) {
    let _ = app.emit(SERIAL_STATE, SerialState { open, port, error });
}

/// Enumerate serial ports, USB adapters first.
pub fn list_ports() -> Result<Vec<PortInfo>, String> {
    let ports = serialport::available_ports().map_err(|e| format!("Could not list serial ports: {e}"))?;
    let names: Vec<String> = ports.iter().map(|p| p.port_name.clone()).collect();
    let keep = preferred_port_names(&names);
    let mut out: Vec<PortInfo> = ports
        .into_iter()
        .filter(|p| keep.contains(&p.port_name))
        .map(|p| PortInfo {
            description: match &p.port_type {
                SerialPortType::UsbPort(usb) => {
                    let text = [usb.product.as_deref(), usb.manufacturer.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                    Some(if text.is_empty() { format!("USB {:04x}:{:04x}", usb.vid, usb.pid) } else { text })
                }
                SerialPortType::BluetoothPort => Some("Bluetooth".into()),
                SerialPortType::PciPort => Some("PCI".into()),
                SerialPortType::Unknown => None,
            },
            name: p.port_name,
        })
        .collect();
    out.sort_by_key(|p| (p.description.is_none(), p.name.clone()));
    Ok(out)
}

/// macOS exposes every serial device twice: `/dev/tty.*` (dial-in, whose open
/// blocks until carrier detect) and `/dev/cu.*` (call-out, opens immediately).
/// A UART console wants `cu.*`, so hide a `tty.*` whose `cu.*` twin exists.
fn preferred_port_names(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter(|n| match n.strip_prefix("/dev/tty.") {
            Some(rest) => !names.iter().any(|m| m == &format!("/dev/cu.{rest}")),
            None => true,
        })
        .cloned()
        .collect()
}

struct PortSettings {
    data_bits: DataBits,
    parity: Parity,
    stop_bits: StopBits,
    flow_control: FlowControl,
}

impl TryFrom<&SerialConfig> for PortSettings {
    type Error = String;

    fn try_from(c: &SerialConfig) -> Result<Self, String> {
        if c.baud_rate == 0 {
            return Err("Baud rate must be greater than 0".into());
        }
        Ok(Self {
            data_bits: match c.data_bits {
                5 => DataBits::Five,
                6 => DataBits::Six,
                7 => DataBits::Seven,
                8 => DataBits::Eight,
                n => return Err(format!("Unsupported data bits: {n}")),
            },
            parity: match c.parity.as_str() {
                "none" => Parity::None,
                "odd" => Parity::Odd,
                "even" => Parity::Even,
                p => return Err(format!("Unsupported parity: {p}")),
            },
            stop_bits: match c.stop_bits {
                1 => StopBits::One,
                2 => StopBits::Two,
                n => return Err(format!("Unsupported stop bits: {n}")),
            },
            flow_control: match c.flow_control.as_str() {
                "none" => FlowControl::None,
                "software" => FlowControl::Software,
                "hardware" => FlowControl::Hardware,
                f => return Err(format!("Unsupported flow control: {f}")),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_dial_in_twin_of_call_out_port() {
        let names: Vec<String> = [
            "/dev/tty.usbserial-1410",
            "/dev/cu.usbserial-1410",
            "/dev/tty.only-dial-in",
            "COM3",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            preferred_port_names(&names),
            vec!["/dev/cu.usbserial-1410", "/dev/tty.only-dial-in", "COM3"]
        );
    }

    /// End-to-end over a pseudo-terminal: the pty slave stands in for the
    /// USB-serial adapter and the test drives the "device" side via the master.
    /// (macOS rejects baud ioctls on ptys, so the slave is started directly.)
    #[cfg(unix)]
    #[test]
    fn pty_round_trip_and_hangup() {
        use std::sync::mpsc;
        use tauri::Listener;

        let pty = || {
            let (master, mut slave) = serialport::TTYPort::pair().unwrap();
            slave.set_timeout(READ_TIMEOUT).unwrap();
            (master, Box::new(slave) as Box<dyn SerialPort>)
        };
        let (mut master, slave) = pty();

        let app = tauri::test::mock_app();
        app.manage(SerialManager::default());
        let handle = app.handle().clone();
        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>();
        handle.listen(SERIAL_DATA, move |e| {
            data_tx.send(serde_json::from_str(e.payload()).unwrap()).unwrap();
        });
        let (state_tx, state_rx) = mpsc::channel::<serde_json::Value>();
        handle.listen(SERIAL_STATE, move |e| {
            state_tx.send(serde_json::from_str(e.payload()).unwrap()).unwrap();
        });
        let mgr = handle.state::<SerialManager>();
        let wait = Duration::from_secs(3);

        mgr.start(&handle, "pty", slave).unwrap();
        assert_eq!(state_rx.recv_timeout(wait).unwrap()["open"], true);
        assert!(mgr.state().open);

        // Device → app: a burst arrives batched, in order.
        master.write_all(b"U-Boot 2024.01\r\nboot> ").unwrap();
        let mut got = Vec::new();
        while got.len() < 22 {
            got.extend(data_rx.recv_timeout(wait).unwrap());
        }
        assert_eq!(got, b"U-Boot 2024.01\r\nboot> ");

        // App → device.
        mgr.write(b"printenv\r\n").unwrap();
        let mut buf = [0u8; 10];
        master.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"printenv\r\n");

        // Re-open the same device (e.g. baud change) releases it cleanly first.
        let (mut master2, slave2) = pty();
        mgr.start(&handle, "pty2", slave2).unwrap();
        assert_eq!(state_rx.recv_timeout(wait).unwrap()["open"], false);
        assert_eq!(state_rx.recv_timeout(wait).unwrap()["open"], true);

        drop(master);
        master2.write_all(b"ok").unwrap();
        assert_eq!(data_rx.recv_timeout(wait).unwrap(), b"ok");

        // Device disappears (adapter unplugged): the session closes with an error.
        drop(master2);
        let closed = state_rx.recv_timeout(wait).unwrap();
        assert_eq!(closed["open"], false);
        assert!(closed["error"].is_string(), "{closed}");
        assert!(!mgr.state().open);
        assert!(mgr.write(b"x").is_err());
    }

    #[test]
    fn rejects_invalid_line_settings() {
        let ok = SerialConfig {
            baud_rate: 115200,
            data_bits: 8,
            parity: "none".into(),
            stop_bits: 1,
            flow_control: "none".into(),
        };
        assert!(PortSettings::try_from(&ok).is_ok());
        for bad in [
            SerialConfig { baud_rate: 0, ..ok.clone() },
            SerialConfig { data_bits: 9, ..ok.clone() },
            SerialConfig { parity: "mark".into(), ..ok.clone() },
            SerialConfig { stop_bits: 3, ..ok.clone() },
            SerialConfig { flow_control: "xon".into(), ..ok.clone() },
        ] {
            assert!(PortSettings::try_from(&bad).is_err());
        }
    }
}
