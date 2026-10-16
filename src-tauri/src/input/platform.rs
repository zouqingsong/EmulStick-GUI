//! Platform cursor control for relative-mouse capture.
//!
//! On lock + mouse passthrough we keep the OS cursor stationary while receiving
//! movement deltas.
//!
//! The two platforms get their relative deltas differently:
//! * **macOS** — the vendored rdev patch makes `MouseMove { x, y }` carry the
//!   HID delta directly (`MOUSE_EVENT_DELTA_X/Y`, see `vendor/rdev/PATCH.md`),
//!   and `CGAssociateMouseAndMouseCursorPosition(false)` freezes the cursor.
//! * **Windows** — rdev's low-level hook reports the **absolute** cursor point
//!   (`vendor/rdev/src/windows/common.rs`). So [`relative_delta`] turns that
//!   absolute point into a delta against a fixed screen-centre anchor. The hook
//!   allows the initial warp to that anchor, then consumes physical movements.
//!   Inside a Remote Desktop session the client reports its own absolute
//!   pointer and ignores our warps, so there we diff successive points instead.

/// Enable (`true`) or release (`false`) relative-mouse capture: freeze the
/// system cursor and decouple it from the physical mouse.
#[cfg(target_os = "macos")]
pub fn set_cursor_capture(capture: bool) {
    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGAssociateMouseAndMouseCursorPosition(connected: i32) -> i32;
        fn CGWarpMouseCursorPosition(new_cursor_position: CGPoint) -> i32;
        fn CGMainDisplayID() -> u32;
        fn CGDisplayPixelsWide(display: u32) -> usize;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
        fn CGDisplayHideCursor(display: u32) -> i32;
        fn CGDisplayShowCursor(display: u32) -> i32;
    }
    unsafe {
        let display = CGMainDisplayID();
        if capture {
            // Centre the cursor first so it isn't frozen against a screen edge,
            // then decouple it from the mouse and hide it.
            let x = CGDisplayPixelsWide(display) as f64 / 2.0;
            let y = CGDisplayPixelsHigh(display) as f64 / 2.0;
            CGWarpMouseCursorPosition(CGPoint { x, y });
            CGAssociateMouseAndMouseCursorPosition(0);
            CGDisplayHideCursor(display);
        } else {
            CGAssociateMouseAndMouseCursorPosition(1);
            CGDisplayShowCursor(display);
        }
    }
}

/// Convert a `MouseMove` event's coordinates into a relative HID delta.
///
/// On macOS the vendored rdev patch already delivers deltas, so this is the
/// identity. (Windows overrides this — see the `target_os = "windows"` variant.)
#[cfg(not(target_os = "windows"))]
pub fn relative_delta(x: f64, y: f64) -> (i32, i32) {
    (x as i32, y as i32)
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
    use winapi::shared::minwindef::{FALSE, TRUE};
    use winapi::shared::windef::POINT;
    use winapi::um::winuser::{
        GetCursorPos, GetMonitorInfoW, GetSystemMetrics, MonitorFromPoint, SetCursorPos,
        ShowCursor, MONITORINFO, MONITOR_DEFAULTTONEAREST, SM_CXSCREEN, SM_CYSCREEN,
        SM_REMOTESESSION,
    };

    /// Whether relative-mouse capture is active. Gates [`super::relative_delta`]
    /// so absolute points are only turned into deltas while we're capturing, and
    /// guards `ShowCursor` so its display counter stays balanced (one hide per
    /// enable, one show per disable) across the many idempotent disable calls.
    static CAPTURED: AtomicBool = AtomicBool::new(false);
    /// Fixed anchor in virtual-desktop coordinates while local moves are consumed.
    static CENTER_X: AtomicI32 = AtomicI32::new(0);
    static CENTER_Y: AtomicI32 = AtomicI32::new(0);
    /// Last absolute point seen, used for deltas inside a Remote Desktop session.
    /// `i32::MIN` means "not seeded yet" so the first event after capture is not a jump.
    static LAST_X: AtomicI32 = AtomicI32::new(i32::MIN);
    static LAST_Y: AtomicI32 = AtomicI32::new(i32::MIN);

    fn is_remote_session() -> bool {
        unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
    }

    /// Centre of the monitor currently under the cursor, in virtual-desktop
    /// coordinates (so `SetCursorPos` warps correctly on multi-monitor setups).
    /// Falls back to the primary monitor's centre if the queries fail.
    unsafe fn monitor_center_under_cursor() -> (i32, i32) {
        let primary = || (GetSystemMetrics(SM_CXSCREEN) / 2, GetSystemMetrics(SM_CYSCREEN) / 2);
        let mut pt = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut pt) == 0 {
            return primary();
        }
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi) == 0 {
            return primary();
        }
        (
            (mi.rcMonitor.left + mi.rcMonitor.right) / 2,
            (mi.rcMonitor.top + mi.rcMonitor.bottom) / 2,
        )
    }

    pub fn set_cursor_capture(capture: bool) {
        unsafe {
            if capture {
                if CAPTURED.load(Ordering::SeqCst) {
                    return;
                }
                let (cx, cy) = monitor_center_under_cursor();
                CENTER_X.store(cx, Ordering::SeqCst);
                CENTER_Y.store(cy, Ordering::SeqCst);
                LAST_X.store(i32::MIN, Ordering::SeqCst);
                LAST_Y.store(i32::MIN, Ordering::SeqCst);
                CAPTURED.store(true, Ordering::SeqCst);
                if !is_remote_session() {
                    SetCursorPos(cx, cy);
                }
                // Best-effort hide. ShowCursor only reliably hides over our own
                // process's windows; with the cursor pinned at centre and all
                // moves consumed this is cosmetic, so the limitation is benign.
                ShowCursor(FALSE);
            } else {
                // Idempotent: disable runs from lock-exit, passthrough changes,
                // emergency unlock, and the panic/exit hooks — only undo once.
                if !CAPTURED.swap(false, Ordering::SeqCst) {
                    return;
                }
                ShowCursor(TRUE);
            }
        }
    }

    fn local_delta(x: f64, y: f64, cx: i32, cy: i32) -> (i32, i32) {
        (x as i32 - cx, y as i32 - cy)
    }

    pub fn pass_anchor_move(x: f64, y: f64) -> bool {
        CAPTURED.load(Ordering::SeqCst)
            && !is_remote_session()
            && local_delta(x, y, CENTER_X.load(Ordering::SeqCst), CENTER_Y.load(Ordering::SeqCst)) == (0, 0)
    }

    /// Local moves are consumed, so every physical event starts at the anchor.
    pub fn relative_delta(x: f64, y: f64) -> (i32, i32) {
        if !CAPTURED.load(Ordering::SeqCst) {
            return (0, 0);
        }
        if is_remote_session() {
            let (x, y) = (x as i32, y as i32);
            let (px, py) = (LAST_X.swap(x, Ordering::SeqCst), LAST_Y.swap(y, Ordering::SeqCst));
            if px == i32::MIN || py == i32::MIN {
                return (0, 0);
            }
            return (x - px, y - py);
        }
        let cx = CENTER_X.load(Ordering::SeqCst);
        let cy = CENTER_Y.load(Ordering::SeqCst);
        local_delta(x, y, cx, cy)
    }

    #[cfg(test)]
    mod tests {
        use super::local_delta;

        #[test]
        fn anchor_warp_has_no_forwarded_motion() {
            assert_eq!(local_delta(960.0, 540.0, 960, 540), (0, 0));
        }

        #[test]
        fn consumed_moves_keep_a_fixed_origin() {
            for _ in 0..3 {
                assert_eq!(local_delta(966.0, 552.0, 960, 540), (6, 12));
            }
            assert_eq!(local_delta(958.0, 543.0, 960, 540), (-2, 3));
            assert_eq!(local_delta(-954.0, 552.0, -960, 540), (6, 12));
        }
    }
}

#[cfg(target_os = "windows")]
pub use win::{pass_anchor_move, relative_delta, set_cursor_capture};

#[cfg(not(target_os = "windows"))]
pub fn pass_anchor_move(_x: f64, _y: f64) -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_cursor_capture(_capture: bool) {}

/// Whether this process is trusted for Accessibility — i.e. allowed to install
/// a `CGEventTap`/`rdev::grab` (plan §5). Without it the global input hook
/// silently fails, so the UI checks this before entering lock and guides the
/// operator to grant it.
///
/// When `prompt` is true and the process is not yet trusted, macOS shows its
/// "<app> would like to control this computer" dialog and adds the app to the
/// Accessibility list — the onboarding entry point.
#[cfg(target_os = "macos")]
pub fn accessibility_trusted(prompt: bool) -> bool {
    use std::ffi::c_void;
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFDictionaryRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            num_values: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFDictionaryRef;
        fn CFRelease(cf: CFTypeRef);
    }
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        // Returns a `Boolean` (unsigned char); `NULL` options == AXIsProcessTrusted().
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
    }

    unsafe {
        let options: CFDictionaryRef = if prompt {
            let key: CFTypeRef = kAXTrustedCheckOptionPrompt;
            let value: CFTypeRef = kCFBooleanTrue;
            CFDictionaryCreate(
                ptr::null(),
                &key,
                &value,
                1,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            )
        } else {
            ptr::null()
        };
        let trusted = AXIsProcessTrustedWithOptions(options) != 0;
        if !options.is_null() {
            CFRelease(options);
        }
        trusted
    }
}

/// Non-macOS platforms have no equivalent gate (Windows low-level hooks need no
/// entitlement), so the process is always "trusted" to capture input.
#[cfg(not(target_os = "macos"))]
pub fn accessibility_trusted(_prompt: bool) -> bool {
    true
}
