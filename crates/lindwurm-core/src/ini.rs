//! Import of Mouse.ini from the vendor's Windows software.
//!
//! Each of the five [GROUPn] sections is one profile; a profile exported by
//! the vendor software (*.conf) has the same keys in a single [Profile]
//! section. ButtonAssigned1-11 are 84-byte records in hex: CC, function type,
//! step count, 26 entries of [flags/delay, usage, unused], and a checksum byte
//! (sum of the others).
//! Function types and the record layout come from a static analysis of the
//! vendor's hid.exe; the factory file only contains the simple types, so
//! single keys and recorded macros are decoded from that analysis alone.

use std::collections::BTreeMap;

use crate::config::{build_layout, Config, BUTTONS, LED_BRIGHTNESS, LED_EFFECT};
use crate::keys::key_name;
use crate::{err, Result};

const RECORD_LEN: usize = 84;
const NUM_GROUPS: usize = 5;

/// Vendor function types that map onto a named action.
const TYPES: &[(u8, &str)] = &[
    (0x01, "left-click"),
    (0x02, "right-click"),
    (0x03, "middle-click"),
    (0x04, "double-click"),
    (0x07, "none"),
    (0x09, "led-switch"),
    (0x0A, "forward"),
    (0x0B, "back"),
    (0x0C, "dpi-up"),
    (0x0D, "dpi-down"),
    (0x20, "play-pause"),
    (0x21, "stop"),
    (0x22, "previous"),
    (0x23, "next"),
    (0x25, "volume-up"),
    (0x26, "volume-down"),
    (0x27, "mute"),
    (0x32, "scroll-left"),
    (0x33, "scroll-right"),
    (0x40, "cut"),
    (0x41, "copy"),
    (0x42, "paste"),
    (0x43, "undo"),
    (0x44, "select-all"),
    (0x45, "find"),
    (0x46, "new"),
    (0x47, "print"),
    (0x48, "save"),
    (0x50, "swap-window"),
    (0x51, "close-window"),
    (0x52, "lock-pc"),
    (0x53, "explorer"),
    (0x54, "run"),
    (0x55, "show-desktop"),
    (0x61, "profile-switch"),
];

const FACTORY_DPI: u8 = 3;
const FACTORY_COLOR: &str = "#00ff00";
const FACTORY_LED_STATE: u8 = 4;
const FACTORY_FLASH_STATE: u8 = 4;

pub struct Imported {
    pub group: usize,
    /// Section of the vendor file, e.g. "GROUP0" or "Profile".
    pub section: String,
    pub toml: String,
    pub warnings: Vec<String>,
    pub same_as_factory: bool,
}

fn sections(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
        } else if let Some((k, v)) = line.split_once('=') {
            out.entry(current.clone())
                .or_default()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

fn decode_record(hex: &str) -> Result<[u8; RECORD_LEN]> {
    if hex.len() != RECORD_LEN * 2 {
        return Err(err!(
            "has {} hex digits, expected {}",
            hex.len(),
            RECORD_LEN * 2
        ));
    }
    let mut r = [0u8; RECORD_LEN];
    for (i, b) in r.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).map_err(|_| err!("is not hex"))?;
    }
    // Most files mark records with CC; some (seen in a Mouse.ini from 2025,
    // GROUP0 only) use 00 with otherwise identical content.
    if r[0] != 0xCC && r[0] != 0x00 {
        return Err(err!("does not start with CC or 00"));
    }
    let sum = r[..RECORD_LEN - 1]
        .iter()
        .fold(0u8, |a, b| a.wrapping_add(*b));
    if sum != r[RECORD_LEN - 1] {
        return Err(err!("has a bad checksum"));
    }
    Ok(r)
}

/// Macro entries as steps in profile syntax. Delays are in the device's
/// 50 ms units; the vendor software copies flags and usage unchanged.
fn macro_steps(r: &[u8; RECORD_LEN]) -> Vec<String> {
    let count = usize::from(r[2]).min((RECORD_LEN - 4) / 3);
    let mut steps = Vec::new();
    for i in 0..count {
        let (flags, usage) = (r[3 + 3 * i], r[4 + 3 * i]);
        let sign = if flags & 0x80 != 0 { "-" } else { "+" };
        steps.push(format!("{sign}{}", key_name(usage)));
        let delay = u32::from(flags & 0x7F) * 50;
        if delay > 0 {
            steps.push(format!("{delay}ms"));
        }
    }
    steps
}

fn group_value(group: &BTreeMap<String, String>, key: &str) -> Option<u32> {
    group.get(key).and_then(|v| v.parse().ok())
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Converts every group of a Mouse.ini into profile text extending "factory".
pub fn import(text: &str, source: &str) -> Result<Vec<Imported>> {
    let sec = sections(text);
    let active = sec
        .get("Setting")
        .and_then(|s| s.get("Profile"))
        .and_then(|v| v.parse::<usize>().ok());
    let mut out = Vec::new();
    let mut found: Vec<(usize, String)> = (0..NUM_GROUPS)
        .map(|g| (g, format!("GROUP{g}")))
        .filter(|(_, s)| sec.contains_key(s))
        .collect();
    if found.is_empty() && sec.contains_key("Profile") {
        found.push((0, "Profile".into()));
    }
    for (g, sname) in found {
        let group = &sec[&sname];
        let mut warnings = Vec::new();
        let mut head = format!("# Imported from {source}, [{sname}]");
        if active == Some(g) {
            head += " (the active profile there)";
        }
        head += ".\nextends = \"factory\"\n";
        let mut settings = String::new();
        let mut led = String::new();
        let mut buttons = String::new();
        let mut macros = String::new();

        if let Some(d) = group_value(group, "DPI") {
            if (1..=7).contains(&d) && d != u32::from(FACTORY_DPI) {
                settings += &format!("dpi = {d}\n");
            }
        }
        if let Some(c) = group_value(group, "LedColor") {
            let color = format!(
                "#{:02x}{:02x}{:02x}",
                c & 0xFF,
                (c >> 8) & 0xFF,
                (c >> 16) & 0xFF
            );
            if color != FACTORY_COLOR {
                led += &format!("color      = {}   # LedColor={c}\n", quote(&color));
            }
        }
        if let Some(s) = group_value(group, "LedState") {
            if (1..=4).contains(&s) && s != u32::from(FACTORY_LED_STATE) {
                let name = LED_BRIGHTNESS[s as usize - 1];
                led += &format!("brightness = {}   # LedState={s}\n", quote(name));
            }
        }
        if let Some(f) = group_value(group, "FlashState") {
            if (1..=4).contains(&f) && f != u32::from(FACTORY_FLASH_STATE) {
                led += &format!(
                    "effect     = {}   # FlashState={f}\n",
                    quote(LED_EFFECT[f as usize - 1])
                );
            }
        }
        // The vendor software offers three rates and sends 04, 02 or 01.
        if let Some(p) = group_value(group, "PollingRate") {
            match [250, 500, 1000].get((p as usize).wrapping_sub(1)) {
                Some(500) => {}
                Some(hz) => settings += &format!("polling-rate = {hz}   # PollingRate={p}\n"),
                None => warnings.push(format!("{sname} PollingRate={p} is unknown, left out")),
            }
        }
        // MouseSpeedX/Y only reach the mouse with XYSensitivityEnabled=1.
        let mut sensitivity = String::new();
        if group_value(group, "XYSensitivityEnabled") == Some(1) {
            for (axis, key) in [("x", "MouseSpeedX"), ("y", "MouseSpeedY")] {
                if let Some(v) = group_value(group, key) {
                    if (1..=9).contains(&v) {
                        sensitivity += &format!("{axis} = {v}   # {key}={v}\n");
                    }
                }
            }
        }

        for (i, (name, _, default)) in BUTTONS.iter().take(11).enumerate() {
            let entry = format!("ButtonAssigned{}", i + 1);
            let Some(hex) = group.get(&entry) else {
                continue;
            };
            let r = match decode_record(hex) {
                Ok(r) => r,
                Err(e) => {
                    warnings.push(format!("{sname} {entry} {e}, kept the factory setting"));
                    continue;
                }
            };
            let ty = r[1];
            let (action, note) = match ty {
                0x05 => {
                    // Usage position taken from the analysis; no sample file had a single key.
                    let usage = if r[4] != 0 { r[4] } else { r[3] };
                    (format!("key:{}", key_name(usage)), String::new())
                }
                0x06 => {
                    let steps = macro_steps(&r);
                    let mname = format!("button{}", i + 1);
                    let mc = crate::config::MacroConfig {
                        repeat: 1,
                        steps: steps.clone(),
                    };
                    match crate::config::check_macro(&mc) {
                        Ok(_) => {
                            let list: Vec<String> = steps.iter().map(|s| quote(s)).collect();
                            macros +=
                                &format!("\n[macros.{mname}]\nsteps = [{}]\n", list.join(", "));
                            (format!("macro:{mname}"), String::new())
                        }
                        Err(e) => {
                            warnings.push(format!("{sname} {entry}: macro not usable ({e})"));
                            let list: Vec<String> = steps.iter().map(|s| quote(s)).collect();
                            macros += &format!(
                                "\n# Macro of {entry} could not be used: {e}\n# [macros.{mname}]\n# steps = [{}]\n",
                                list.join(", ")
                            );
                            (
                                "none".to_string(),
                                " (macro not usable, see below)".to_string(),
                            )
                        }
                    }
                }
                0x30 | 0x31 => (
                    format!("raw:00{:02x}0000", if ty == 0x30 { 0xF8 } else { 0xF9 }),
                    " (function of unknown meaning, copied as sent by the vendor software)"
                        .to_string(),
                ),
                _ => match TYPES.iter().find(|(t, _)| *t == ty) {
                    Some((_, a)) => (a.to_string(), String::new()),
                    None => {
                        warnings.push(format!(
                            "{sname} {entry}: function type 0x{ty:02x} has no known mouse code, set to none"
                        ));
                        (
                            "none".to_string(),
                            format!(" (vendor function type 0x{ty:02x})"),
                        )
                    }
                },
            };
            if action != *default || !note.is_empty() {
                buttons += &format!(
                    "{name:<12} = {}   # {entry}, type 0x{ty:02x}{note}\n",
                    quote(&action)
                );
            }
        }

        let mut toml = head;
        if !settings.is_empty() {
            toml += &format!("\n{settings}");
        }
        if !led.is_empty() {
            toml += &format!("\n[led]\n{led}");
        }
        if !sensitivity.is_empty() {
            toml += &format!("\n[sensitivity]\n{sensitivity}");
        }
        if !buttons.is_empty() {
            toml += &format!("\n[buttons]\n{buttons}");
        }
        toml += &macros;

        let c =
            Config::parse(&toml).map_err(|e| err!("{sname}: generated profile is invalid: {e}"))?;
        build_layout(&c).map_err(|e| err!("{sname}: generated profile is invalid: {e}"))?;
        let same_as_factory =
            led.is_empty() && buttons.is_empty() && sensitivity.is_empty() && settings.is_empty();
        out.push(Imported {
            group: g,
            section: sname,
            toml,
            warnings,
            same_as_factory,
        });
    }
    if out.is_empty() {
        return Err(err!(
            "no [GROUP0]-[GROUP4] or [Profile] section found, is this a Mouse.ini or an exported .conf?"
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(bytes: &[u8]) -> String {
        let mut r = [0u8; RECORD_LEN];
        r[..bytes.len()].copy_from_slice(bytes);
        r[RECORD_LEN - 1] = r[..RECORD_LEN - 1]
            .iter()
            .fold(0u8, |a, b| a.wrapping_add(*b));
        r.iter().map(|b| format!("{b:02X}")).collect()
    }

    fn factory_group() -> Vec<(String, String)> {
        let types = [
            0x01, 0x02, 0x03, 0x0A, 0x0B, 0x32, 0x33, 0x0C, 0x0D, 0x09, 0x04,
        ];
        let mut v: Vec<(String, String)> = types
            .iter()
            .enumerate()
            .map(|(i, t)| (format!("ButtonAssigned{}", i + 1), record(&[0xCC, *t])))
            .collect();
        for (k, val) in [
            ("LedState", "4"),
            ("FlashState", "4"),
            ("LedColor", "65280"),
            ("PollingRate", "2"),
            ("DPI", "3"),
        ] {
            v.push((k.into(), val.into()));
        }
        v
    }

    fn ini(groups: &[Vec<(String, String)>]) -> String {
        let mut s = "[Setting]\r\nProfile=1\r\nMouseButtons=11\r\n".to_string();
        for (g, entries) in groups.iter().enumerate() {
            s += &format!("[GROUP{g}]\r\n");
            for (k, v) in entries {
                s += &format!("{k}={v}\r\n");
            }
        }
        s
    }

    #[test]
    fn factory_ini_matches_factory_profile() {
        // Same record layout as the shipped file: CC 01 00.. checksum CD.
        assert!(record(&[0xCC, 0x01]).ends_with("CD"));
        let out = import(&ini(&[factory_group()]), "Mouse.ini").unwrap();
        assert_eq!(out.len(), 1);
        assert!(out[0].same_as_factory, "{}", out[0].toml);
        assert!(out[0].warnings.is_empty());
    }

    #[test]
    fn records_starting_with_00() {
        let g: Vec<(String, String)> = factory_group()
            .into_iter()
            .map(|(k, v)| match v.strip_prefix("CC") {
                Some(rest) if k.starts_with("ButtonAssigned") => {
                    // Lead byte CC -> 00 lowers the checksum by CC.
                    let mut r = format!("00{rest}");
                    let sum = u8::from_str_radix(&r[r.len() - 2..], 16).unwrap();
                    r.truncate(r.len() - 2);
                    r += &format!("{:02X}", sum.wrapping_sub(0xCC));
                    (k, r)
                }
                _ => (k, v),
            })
            .collect();
        let out = import(&ini(&[g]), "Mouse.ini").unwrap();
        assert!(out[0].warnings.is_empty(), "{:?}", out[0].warnings);
        assert!(out[0].same_as_factory, "{}", out[0].toml);
    }

    #[test]
    fn polling_rate_and_sensitivity() {
        let mut g = factory_group();
        g.retain(|(k, _)| k != "PollingRate");
        for (k, v) in [
            ("PollingRate", "3"),
            ("XYSensitivityEnabled", "1"),
            ("MouseSpeedX", "4"),
            ("MouseSpeedY", "10"),
        ] {
            g.push((k.into(), v.into()));
        }
        let out = import(&ini(&[g]), "Mouse.ini").unwrap();
        let c = Config::parse(&out[0].toml).unwrap();
        assert_eq!(c.polling_rate, Some(1000), "{}", out[0].toml);
        assert_eq!((c.sensitivity.x, c.sensitivity.y), (Some(4), None));
        assert!(!out[0].same_as_factory);
    }

    #[test]
    fn exported_conf() {
        let mut g = factory_group();
        g.retain(|(k, _)| k != "DPI");
        g.push(("DPI".into(), "5".into()));
        let mut s = "[Profile]\r\n".to_string();
        for (k, v) in &g {
            s += &format!("{k}={v}\r\n");
        }
        let out = import(&s, "profile.conf").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].section, "Profile");
        assert!(out[0].toml.contains("dpi = 5"), "{}", out[0].toml);
    }

    #[test]
    fn custom_group() {
        let mut g = factory_group();
        let set = |g: &mut Vec<(String, String)>, k: &str, v: String| {
            g.retain(|(x, _)| x != k);
            g.push((k.to_string(), v));
        };
        set(
            &mut g,
            "ButtonAssigned10",
            record(&[0xCC, 0x05, 0x01, 0x00, 0x0A]),
        );
        // Shift down, h tap with 100 ms after release, shift up.
        set(
            &mut g,
            "ButtonAssigned11",
            record(&[
                0xCC, 0x06, 0x04, 0x00, 0xE1, 0, 0x00, 0x0B, 0, 0x82, 0x0B, 0, 0x80, 0xE1,
            ]),
        );
        set(&mut g, "ButtonAssigned6", record(&[0xCC, 0x27]));
        set(&mut g, "ButtonAssigned7", record(&[0xCC, 0x60]));
        set(&mut g, "ButtonAssigned8", "CC0C00".into());
        set(&mut g, "LedColor", "255".into());
        set(&mut g, "DPI", "5".into());
        let out = import(&ini(&[factory_group(), g]), "Mouse.ini").unwrap();
        let t = &out[1].toml;
        assert!(t.contains("(the active profile there)"), "{t}");
        assert!(t.contains("dpi = 5"), "{t}");
        assert!(t.contains("color      = \"#ff0000\""), "{t}");
        assert!(t.contains("side-right   = \"key:g\""), "{t}");
        assert!(t.contains("thumb-bottom = \"macro:button11\""), "{t}");
        assert!(
            t.contains("steps = [\"+lshift\", \"+h\", \"-h\", \"100ms\", \"-lshift\"]"),
            "{t}"
        );
        assert!(t.contains("tilt-left    = \"mute\""), "{t}");
        assert!(t.contains("tilt-right   = \"none\""), "{t}");
        assert_eq!(out[1].warnings.len(), 2, "{:?}", out[1].warnings);
        assert!(!out[1].same_as_factory);
        let c = Config::parse(t).unwrap();
        assert_eq!(c.extends.as_deref(), Some("factory"));
    }

    #[test]
    fn not_an_ini() {
        assert!(import("hello", "x").is_err());
    }
}
