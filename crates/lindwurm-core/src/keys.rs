// HID keyboard usage IDs. They name physical key positions of a US layout,
// so on a German layout "y" and "z" are swapped and symbols differ.

const NAMED: &[(&str, u8)] = &[
    ("enter", 0x28),
    ("esc", 0x29),
    ("backspace", 0x2A),
    ("tab", 0x2B),
    ("space", 0x2C),
    ("minus", 0x2D),
    ("equal", 0x2E),
    ("leftbrace", 0x2F),
    ("rightbrace", 0x30),
    ("backslash", 0x31),
    ("semicolon", 0x33),
    ("apostrophe", 0x34),
    ("grave", 0x35),
    ("comma", 0x36),
    ("dot", 0x37),
    ("slash", 0x38),
    ("capslock", 0x39),
    ("printscreen", 0x46),
    ("scrolllock", 0x47),
    ("pause", 0x48),
    ("insert", 0x49),
    ("home", 0x4A),
    ("pageup", 0x4B),
    ("delete", 0x4C),
    ("end", 0x4D),
    ("pagedown", 0x4E),
    ("right", 0x4F),
    ("left", 0x50),
    ("down", 0x51),
    ("up", 0x52),
    ("numlock", 0x53),
    ("102nd", 0x64),
    ("menu", 0x65),
    ("lctrl", 0xE0),
    ("lshift", 0xE1),
    ("lalt", 0xE2),
    ("lmeta", 0xE3),
    ("rctrl", 0xE4),
    ("rshift", 0xE5),
    ("ralt", 0xE6),
    ("rmeta", 0xE7),
    // Mouse buttons; the vendor software builds its double-click macro from
    // these codes (00 F0 80 F0 ...), the same values used in button slots.
    ("lmb", 0xF0),
    ("rmb", 0xF1),
    ("mmb", 0xF2),
    ("mb4", 0xF3),
    ("mb5", 0xF4),
];

// The keyboard descriptor declares usages up to 0x65; Linux drops anything
// above (verified: F13 is sent by the mouse but never reaches evdev).
pub const KEYBOARD_MAX: u8 = 0x65;

pub fn key_code(name: &str) -> Option<u8> {
    if let Some(&(_, c)) = NAMED.iter().find(|(n, _)| *n == name) {
        return Some(c);
    }
    let bytes = name.as_bytes();
    if bytes.len() == 1 {
        return match bytes[0] {
            c @ b'a'..=b'z' => Some(0x04 + c - b'a'),
            b'0' => Some(0x27),
            c @ b'1'..=b'9' => Some(0x1E + c - b'1'),
            _ => None,
        };
    }
    if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        return match n {
            1..=12 => Some(0x3A + n - 1),
            13..=24 => Some(0x68 + n - 13),
            _ => None,
        };
    }
    name.strip_prefix("0x")
        .and_then(|h| u8::from_str_radix(h, 16).ok())
}

/// Name for a key code as accepted by `key_code`, or "0xNN".
pub fn key_name(code: u8) -> String {
    key_names()
        .into_iter()
        .find(|n| key_code(n) == Some(code))
        .unwrap_or_else(|| format!("0x{code:02x}"))
}

/// All key names accepted by `key_code`, in a sensible order for pickers.
pub fn key_names() -> Vec<String> {
    let mut v: Vec<String> = ('a'..='z').map(String::from).collect();
    v.extend(('1'..='9').chain(['0']).map(String::from));
    v.extend((1..=24).map(|n| format!("f{n}")));
    v.extend(NAMED.iter().map(|(n, _)| n.to_string()));
    v
}

pub fn key_warning(code: u8) -> Option<String> {
    let modifier = (0xE0..=0xE7).contains(&code);
    let mouse = (0xF0..=0xF4).contains(&code);
    (code > KEYBOARD_MAX && !modifier && !mouse).then(|| {
        format!("key 0x{code:02X} is above 0x{KEYBOARD_MAX:02X} and will be dropped by Linux")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(key_code("a"), Some(0x04));
        assert_eq!(key_code("g"), Some(0x0A));
        assert_eq!(key_code("1"), Some(0x1E));
        assert_eq!(key_code("0"), Some(0x27));
        assert_eq!(key_code("f1"), Some(0x3A));
        assert_eq!(key_code("f12"), Some(0x45));
        assert_eq!(key_code("f13"), Some(0x68));
        assert_eq!(key_code("f24"), Some(0x73));
        assert_eq!(key_code("lshift"), Some(0xE1));
        assert_eq!(key_code("0x0a"), Some(0x0A));
        assert_eq!(key_code("f25"), None);
        assert_eq!(key_code("nope"), None);
        assert!(key_warning(0x68).is_some());
        assert!(key_warning(0xE1).is_none());
        assert!(key_warning(0x0A).is_none());
        assert_eq!(key_code("lmb"), Some(0xF0));
        assert_eq!(key_name(0x0A), "g");
        assert_eq!(key_name(0xF1), "rmb");
        assert_eq!(key_name(0x99), "0x99");
        assert!(key_warning(0xF0).is_none());
    }
}
