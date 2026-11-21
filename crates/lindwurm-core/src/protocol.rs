// The Drakonia is a Holtek OEM design shared with the Perixx MX-3000. The
// scrambling follows mx3000control by Gianluca Nitti (MIT) and was confirmed
// against EncodeSecrecy_V11 in the vendor's HIDApi.dll: the key is the
// nibble-swapped string "oNgtEng", the XOR stage there always uses zeros.

pub const FEATURE_REPORT: u8 = 0x07;
pub const NUM_SLOTS: usize = 16;
pub const NUM_MACROS: usize = 16;
pub const MACRO_REPORTS: usize = 12;
pub const MACRO_BYTES: usize = MACRO_REPORTS * 8;
pub const MAX_MACRO_STEPS: usize = (MACRO_BYTES - 2) / 2;

pub type Slot = [u8; 4];

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub slots: [Slot; NUM_SLOTS],
    pub macros: [Vec<u8>; NUM_MACROS],
    pub warnings: Vec<String>,
}

const KEY: [u8; 7] = [0xF6, 0xE4, 0x76, 0x47, 0x54, 0xE6, 0x76];

pub fn scramble(p: &mut [u8; 7]) {
    let mut b = [0u8; 7];
    for i in 0..7 {
        b[i] = p[i].wrapping_add(KEY[i]);
    }
    let first = b[0];
    for i in 0..6 {
        b[i] = (b[i + 1] >> 5) | (b[i] << 3);
    }
    b[6] = (b[6] << 3) | (first >> 5);
    b.swap(0, 3);
    b.swap(2, 6);
    b.swap(4, 5);
    *p = b;
}

pub fn unscramble(p: &mut [u8; 7]) {
    let mut b = *p;
    b.swap(4, 5);
    b.swap(2, 6);
    b.swap(0, 3);
    let mut r = [0u8; 7];
    for i in 0..7 {
        r[i] |= b[i] >> 3;
        r[(i + 1) % 7] |= b[i] << 5;
    }
    for i in 0..7 {
        p[i] = r[i].wrapping_sub(KEY[i]);
    }
}

pub fn scrambled(plain: [u8; 7]) -> [u8; 7] {
    let mut p = plain;
    scramble(&mut p);
    p
}

pub fn describe_layout(l: &Layout, names: &[(&str, usize)]) -> String {
    let mut s = String::new();
    for (name, slot) in names {
        let b = l.slots[*slot];
        s += &format!(
            "slot {slot:2}  {name:<13} {:02X} {:02X} {:02X} {:02X}\n",
            b[0], b[1], b[2], b[3]
        );
    }
    for (i, m) in l.macros.iter().enumerate() {
        if !m.is_empty() {
            let hex: Vec<String> = m.iter().map(|b| format!("{b:02X}")).collect();
            s += &format!("macro {i}  {}\n", hex.join(" "));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pre-scrambled reports hard-coded in mx3000control.
    #[test]
    fn scramble_vectors() {
        let cases: [([u8; 7], [u8; 7]); 5] = [
            (
                [0x03, 0x01, 0, 0, 0, 0, 0],
                [0x3A, 0x2B, 0xB7, 0xCF, 0x33, 0xA7, 0xB2],
            ),
            (
                [0x02, 0x01, 0, 0, 0, 0, 0],
                [0x3A, 0x2B, 0xB7, 0xC7, 0x33, 0xA7, 0xB2],
            ),
            (
                [0x04, 0x01, 0x40, 0, 0, 0, 0],
                [0x3A, 0x2D, 0xB7, 0xD7, 0x33, 0xA7, 0xB2],
            ),
            (
                [0x05, 0x01, 0, 0x06, 0, 0, 0],
                [0x6A, 0x2B, 0xB7, 0xDF, 0x33, 0xA7, 0xB2],
            ),
            ([0; 7], [0x3A, 0x23, 0xB7, 0xB7, 0x33, 0xA7, 0xB2]),
        ];
        for (plain, want) in cases {
            let mut got = plain;
            scramble(&mut got);
            assert_eq!(got, want, "scramble({plain:02X?})");
            unscramble(&mut got);
            assert_eq!(got, plain, "round trip");
        }
    }
}
