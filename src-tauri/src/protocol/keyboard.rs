//! F801 keyboard report encoder (8 bytes, no Report ID).
//!
//! Layout: `[Modifiers, 0x00, Key1, Key2, Key3, Key4, Key5, Key6]`.
//! See `docs/protocol.md` §II.

/// Length of an F801 keyboard report in bytes.
pub const KEYBOARD_REPORT_LEN: usize = 8;

/// Maximum simultaneous HID usage IDs the report can carry (6KRO).
pub const MAX_KEYS: usize = 6;

/// HID "ErrorRollOver" usage id, emitted in every key slot when more than
/// [`MAX_KEYS`] keys are held at once (standard HID behaviour).
const ROLLOVER: u8 = 0x01;

/// Modifier bit masks for byte 0 of the keyboard report.
///
/// Bit order per `docs/protocol.md`: bit0 LeftCtrl … bit7 RightGui.
pub mod modifier {
    pub const LEFT_CTRL: u8 = 1 << 0;
    pub const LEFT_SHIFT: u8 = 1 << 1;
    pub const LEFT_ALT: u8 = 1 << 2;
    pub const LEFT_GUI: u8 = 1 << 3;
    pub const RIGHT_CTRL: u8 = 1 << 4;
    pub const RIGHT_SHIFT: u8 = 1 << 5;
    pub const RIGHT_ALT: u8 = 1 << 6;
    pub const RIGHT_GUI: u8 = 1 << 7;
}

/// Tracks the currently-held modifiers and usage IDs, and renders the full
/// 8-byte report on every change (the dongle expects the complete array, not
/// deltas — see `docs/protocol.md`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyboardState {
    modifiers: u8,
    /// Held usage IDs in press order, so releasing a middle key compacts the
    /// remaining keys toward the low slots exactly as the worked examples show.
    keys: Vec<u8>,
}

impl KeyboardState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set or clear one or more modifier bits (OR-combined mask).
    pub fn set_modifier(&mut self, mask: u8, pressed: bool) {
        if pressed {
            self.modifiers |= mask;
        } else {
            self.modifiers &= !mask;
        }
    }

    /// Press a regular key. No-op if already held (auto-repeat is collapsed).
    pub fn press_key(&mut self, usage: u8) {
        if usage != 0 && !self.keys.contains(&usage) {
            self.keys.push(usage);
        }
    }

    /// Release a regular key. No-op if it was not held.
    pub fn release_key(&mut self, usage: u8) {
        self.keys.retain(|&k| k != usage);
    }

    /// Apply an rdev key event, routing modifiers and regular keys via the
    /// keymap table. Returns `true` if the event mapped to anything (and thus
    /// changed, or could have changed, the report).
    pub fn apply(&mut self, key: &rdev::Key, pressed: bool) -> bool {
        match super::keymap::map_key(key) {
            Some(super::keymap::Mapping::Modifier(mask)) => {
                self.set_modifier(mask, pressed);
                true
            }
            Some(super::keymap::Mapping::Usage(usage)) => {
                if pressed {
                    self.press_key(usage);
                } else {
                    self.release_key(usage);
                }
                true
            }
            None => false,
        }
    }

    /// Clear all held keys and modifiers (used to render the safe all-up state).
    pub fn reset(&mut self) {
        self.modifiers = 0;
        self.keys.clear();
    }

    /// Render the current state as an 8-byte F801 report.
    pub fn report(&self) -> [u8; KEYBOARD_REPORT_LEN] {
        let mut buf = [0u8; KEYBOARD_REPORT_LEN];
        buf[0] = self.modifiers;
        // buf[1] stays 0x00 (reserved).
        if self.keys.len() > MAX_KEYS {
            for slot in buf.iter_mut().skip(2) {
                *slot = ROLLOVER;
            }
        } else {
            for (i, &usage) in self.keys.iter().enumerate() {
                buf[2 + i] = usage;
            }
        }
        buf
    }
}

/// The all-keys-up report (`00 × 8`). Sent on disconnect/lock-exit/panic so no
/// key is left stuck pressed on the host (plan §9).
pub const KEYBOARD_RELEASE_ALL: [u8; KEYBOARD_REPORT_LEN] = [0u8; KEYBOARD_REPORT_LEN];

pub fn panel_report(modifiers: u8, keys: &[u8]) -> Result<[u8; 8], String> {
    if keys.len() > MAX_KEYS || keys.iter().any(|key| !(4..=106).contains(key)) {
        return Err("Use at most six keys with HID usage IDs 4..106".into());
    }
    let mut state = KeyboardState::new();
    state.set_modifier(modifiers, true);
    for &key in keys {
        state.press_key(key);
    }
    Ok(state.report())
}

pub fn text_reports(text: &str, enter: bool) -> Result<Vec<[u8; 8]>, String> {
    if text.len() > 4096 {
        return Err("Text is limited to 4096 bytes per send".into());
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let plain = b"1234567890-=[]\\;'`,./";
    let shifted = b"!@#$%^&*()_+{}|:\"~<>?";
    let usages = [30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46, 47, 48, 49, 51, 52, 53, 54, 55, 56];
    let mut reports = Vec::new();
    for character in normalized.chars().chain(enter.then_some('\n')) {
        let (modifiers, usage) = match character {
            'a'..='z' => (0, character as u8 - b'a' + 4),
            'A'..='Z' => (modifier::LEFT_SHIFT, character as u8 - b'A' + 4),
            ' ' => (0, 44),
            '\n' => (0, 40),
            '\t' => (0, 43),
            character if character.is_ascii() => {
                let byte = character as u8;
                if let Some(index) = plain.iter().position(|&value| value == byte) {
                    (0, usages[index])
                } else if let Some(index) = shifted.iter().position(|&value| value == byte) {
                    (modifier::LEFT_SHIFT, usages[index])
                } else {
                    return Err(format!("Unsupported US-layout character: {character:?}"));
                }
            }
            character => return Err(format!("Unsupported US-layout character: {character:?}")),
        };
        reports.push(panel_report(modifiers, &[usage])?);
        reports.push(KEYBOARD_RELEASE_ALL);
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use super::modifier::*;
    use super::*;

    #[test]
    fn panel_text_us_layout_and_releases() {
        let reports = text_reports("aA!?0", true).unwrap();
        assert_eq!(reports.len(), 12);
        for (index, (modifier, usage)) in [(0, 4), (2, 4), (2, 30), (2, 56), (0, 39), (0, 40)].into_iter().enumerate() {
            assert_eq!(reports[index * 2], panel_report(modifier, &[usage]).unwrap());
            assert_eq!(reports[index * 2 + 1], KEYBOARD_RELEASE_ALL);
        }
    }

    #[test]
    fn panel_text_validates_before_sending() {
        assert!(text_reports("valid then \u{00e9}", false).is_err());
        assert!(text_reports("\0", false).is_err());
        assert!(text_reports(&"x".repeat(4097), false).is_err());
        assert!(panel_report(0, &[4; 7]).is_err());
        assert!(panel_report(0, &[107]).is_err());
    }

    #[test]
    fn panel_text_normalizes_newlines() {
        assert_eq!(text_reports("\r\n\r\n", false).unwrap(), text_reports("\n\n", false).unwrap());
        assert!(text_reports("", false).unwrap().is_empty());
        assert_eq!(text_reports("\t", false).unwrap()[0][2], 43);
        assert_eq!(text_reports("\r", false).unwrap()[0][2], 40);
    }

    /// docs/protocol.md Example 1.1 — Win+Shift+S (Win10 screenshot), using the
    /// *right* Win and *right* Shift keys exactly as documented.
    #[test]
    fn win_shift_s_sequence() {
        let mut kb = KeyboardState::new();

        kb.set_modifier(RIGHT_GUI, true);
        assert_eq!(kb.report(), [0x80, 0, 0, 0, 0, 0, 0, 0]);

        kb.set_modifier(RIGHT_SHIFT, true);
        assert_eq!(kb.report(), [0xA0, 0, 0, 0, 0, 0, 0, 0]);

        kb.press_key(0x16); // "s"
        assert_eq!(kb.report(), [0xA0, 0, 0x16, 0, 0, 0, 0, 0]);

        kb.release_key(0x16);
        assert_eq!(kb.report(), [0xA0, 0, 0, 0, 0, 0, 0, 0]);

        kb.set_modifier(RIGHT_GUI | RIGHT_SHIFT, false);
        assert_eq!(kb.report(), [0x00, 0, 0, 0, 0, 0, 0, 0]);
    }

    /// docs/protocol.md Example 1.2 — Shift+Up+Right, using the *left* Shift.
    /// Verifies that releasing the Up arrow compacts Right into the first slot.
    #[test]
    fn shift_up_right_sequence() {
        let mut kb = KeyboardState::new();

        kb.set_modifier(LEFT_SHIFT, true);
        assert_eq!(kb.report(), [0x02, 0, 0, 0, 0, 0, 0, 0]);

        kb.press_key(0x52); // Up arrow
        assert_eq!(kb.report(), [0x02, 0, 0x52, 0, 0, 0, 0, 0]);

        kb.press_key(0x4F); // Right arrow
        assert_eq!(kb.report(), [0x02, 0, 0x52, 0x4F, 0, 0, 0, 0]);

        kb.release_key(0x52); // release Up — Right compacts down
        assert_eq!(kb.report(), [0x02, 0, 0x4F, 0, 0, 0, 0, 0]);

        kb.release_key(0x4F);
        assert_eq!(kb.report(), [0x02, 0, 0, 0, 0, 0, 0, 0]);

        kb.set_modifier(LEFT_SHIFT, false);
        assert_eq!(kb.report(), [0x00, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn ignores_duplicate_press_and_unknown_release() {
        let mut kb = KeyboardState::new();
        kb.press_key(0x04);
        kb.press_key(0x04); // duplicate ignored
        assert_eq!(kb.report(), [0, 0, 0x04, 0, 0, 0, 0, 0]);
        kb.release_key(0x05); // not held — no-op
        assert_eq!(kb.report(), [0, 0, 0x04, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn more_than_six_keys_reports_rollover() {
        let mut kb = KeyboardState::new();
        for usage in [0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A] {
            kb.press_key(usage);
        }
        assert_eq!(kb.report(), [0, 0, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01]);
    }

    #[test]
    fn reset_clears_everything() {
        let mut kb = KeyboardState::new();
        kb.set_modifier(LEFT_CTRL, true);
        kb.press_key(0x04);
        kb.reset();
        assert_eq!(kb.report(), KEYBOARD_RELEASE_ALL);
    }
}
