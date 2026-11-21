use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::keys::{key_code, key_warning};
use crate::protocol::{Layout, Slot, MAX_MACRO_STEPS, NUM_MACROS};
use crate::{err, Result};

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MacroConfig {
    #[serde(default)]
    pub repeat: u32,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub extends: Option<String>,
    #[serde(default)]
    pub dpi: Option<u8>,
    #[serde(default)]
    pub buttons: BTreeMap<String, String>,
    #[serde(default)]
    pub macros: BTreeMap<String, MacroConfig>,
    #[serde(default)]
    pub led: LedConfig,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LedConfig {
    pub color: Option<String>,
    pub brightness: Option<String>,
    pub effect: Option<String>,
}

// LED commands from the vendor software (0A/0B/0C) and mx3000control; the
// value names follow mx3000control. Verified on a Drakonia Black: brightness
// off..high, effect steady or pulsing (slow and fast differ only slightly).
pub const LED_BRIGHTNESS: &[&str] = &["off", "low", "medium", "high"];
pub const LED_EFFECT: &[&str] = &["steady", "slow", "medium", "fast"];

/// Device level 1-4 of a named LED setting.
pub fn led_level(names: &[&str], value: &str) -> Result<u8> {
    if let Ok(n @ 1..=4) = value.parse::<u8>() {
        return Ok(n);
    }
    names
        .iter()
        .position(|n| *n == value)
        .map(|i| i as u8 + 1)
        .ok_or_else(|| {
            err!(
                "{value:?} is not one of {} or a level 1-4",
                names.join(", ")
            )
        })
}

pub fn parse_color(s: &str) -> Result<[u8; 3]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 {
        return Err(err!("color {s:?} must look like #rrggbb"));
    }
    let mut c = [0u8; 3];
    for (i, b) in c.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[2 * i..2 * i + 2], 16)
            .map_err(|_| err!("color {s:?} must look like #rrggbb"))?;
    }
    Ok(c)
}

impl Config {
    pub fn parse(text: &str) -> Result<Config> {
        let mut c: Config = toml::from_str(text).map_err(|e| err!("{e}"))?;
        let mut buttons = BTreeMap::new();
        for (k, v) in std::mem::take(&mut c.buttons) {
            let name = canonical_button(&k).to_string();
            if buttons.insert(name.clone(), v).is_some() {
                return Err(err!("button {name} is set twice (also as {k})"));
            }
        }
        c.buttons = buttons;
        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(d) = self.dpi {
            if !(1..=7).contains(&d) {
                return Err(err!("dpi must be a level from 1 to 7"));
            }
        }
        if let Some(c) = &self.led.color {
            parse_color(c)?;
        }
        if let Some(b) = &self.led.brightness {
            led_level(LED_BRIGHTNESS, b)?;
        }
        if let Some(e) = &self.led.effect {
            led_level(LED_EFFECT, e)?;
        }
        Ok(())
    }
}

/// Physical button, its slot in the device table and its default action.
/// Slots were mapped by pressing each button with distinct test keys
/// assigned; slots 11-13 belong to no physical button.
/// Defaults are the factory settings (ButtonAssigned1-11 of the vendor's
/// Mouse.ini), so an empty profile behaves like the mouse out of the box.
pub const BUTTONS: &[(&str, usize, &str)] = &[
    ("left", 0, "left-click"),
    ("right", 1, "right-click"),
    ("middle", 2, "middle-click"),
    ("thumb-front", 3, "forward"),
    ("thumb-back", 4, "back"),
    ("tilt-left", 5, "scroll-left"),
    ("tilt-right", 6, "scroll-right"),
    ("dpi-up", 7, "dpi-up"),
    ("dpi-down", 8, "dpi-down"),
    ("side-right", 9, "led-switch"),
    ("thumb-bottom", 10, "double-click"),
    ("wheel-up", 14, "scroll"),
    ("wheel-down", 15, "scroll"),
];

/// Alternative button names, following the Drakonia Black manual.
pub const BUTTON_ALIASES: &[(&str, &str)] = &[
    ("left-button", "left"),
    ("right-button", "right"),
    ("middle-button", "middle"),
    ("illumination", "side-right"),
    ("illumination-key", "side-right"),
];

pub fn canonical_button(name: &str) -> &str {
    BUTTON_ALIASES
        .iter()
        .find(|(a, _)| *a == name)
        .map_or(name, |(_, c)| c)
}

pub const ACTIONS: &[(&str, Slot)] = &[
    ("none", [0, 0, 0, 0]),
    ("scroll", [0, 0, 0, 0]),
    ("left-click", [0x00, 0xF0, 0, 0]),
    ("right-click", [0x00, 0xF1, 0, 0]),
    ("middle-click", [0x00, 0xF2, 0, 0]),
    ("back", [0x00, 0xF3, 0, 0]),
    ("forward", [0x00, 0xF4, 0, 0]),
    ("dpi-up", [0x71, 0x01, 0, 0]),
    ("dpi-down", [0x71, 0x02, 0, 0]),
    ("play-pause", [0x04, 0, 0, 0xCD]),
    ("stop", [0x04, 0, 0, 0xB7]),
    ("previous", [0x04, 0, 0, 0xB6]),
    ("next", [0x04, 0, 0, 0xB5]),
    ("volume-up", [0x04, 0, 0, 0xE9]),
    ("volume-down", [0x04, 0, 0, 0xEA]),
    ("mute", [0x04, 0, 0, 0xE2]),
    ("led-switch", [0x72, 0x01, 0, 0]),
    // The mouse only reports these to the host (input report 03 F3/F4/F6);
    // the vendor's hid.exe turns them into scrolling or a profile change.
    ("scroll-left", [0x04, 0, 0, 0xF3]),
    ("scroll-right", [0x04, 0, 0, 0xF4]),
    ("profile-switch", [0x04, 0, 0, 0xF6]),
];

const HOST_SIDE: &[&str] = &["scroll-left", "scroll-right", "profile-switch"];

/// Alternative action names, following the vendor software.
pub const ACTION_ALIASES: &[(&str, &str)] = &[
    ("click", "left-click"),
    ("menu", "right-click"),
    ("disable", "none"),
    ("led-color-switch", "led-switch"),
];

/// Functions the vendor software builds as macros on the fly.
pub const BUILTIN_MACROS: &[(&str, &[&str])] = &[
    ("double-click", &["lmb", "lmb"]),
    ("cut", &["+lctrl", "x", "-lctrl"]),
    ("copy", &["+lctrl", "c", "-lctrl"]),
    ("paste", &["+lctrl", "v", "-lctrl"]),
    ("undo", &["+lctrl", "z", "-lctrl"]),
    ("select-all", &["+lctrl", "a", "-lctrl"]),
    ("find", &["+lctrl", "f", "-lctrl"]),
    ("new", &["+lctrl", "n", "-lctrl"]),
    ("print", &["+lctrl", "p", "-lctrl"]),
    ("save", &["+lctrl", "s", "-lctrl"]),
    ("swap-window", &["+lalt", "tab", "-lalt"]),
    ("close-window", &["+lalt", "f4", "-lalt"]),
    ("lock-pc", &["+lmeta", "l", "-lmeta"]),
    ("explorer", &["+lmeta", "e", "-lmeta"]),
    ("run", &["+lmeta", "r", "-lmeta"]),
    ("show-desktop", &["+lmeta", "d", "-lmeta"]),
];

pub fn canonical_action(name: &str) -> &str {
    ACTION_ALIASES
        .iter()
        .find(|(a, _)| *a == name)
        .map_or(name, |(_, c)| c)
}

/// HID consumer page usages with a Linux keycode in hid-input. The device
/// passes all 16 bits (verified with 0x0192 -> KEY_CALC); its descriptor
/// allows up to 0x2FFF.
pub const CONSUMER_CODES: &[(&str, u16)] = &[
    ("play-pause", 0x0CD),
    ("stop", 0x0B7),
    ("previous", 0x0B6),
    ("next", 0x0B5),
    ("fast-forward", 0x0B3),
    ("rewind", 0x0B4),
    ("record", 0x0B2),
    ("eject", 0x0B8),
    ("mute", 0x0E2),
    ("volume-up", 0x0E9),
    ("volume-down", 0x0EA),
    ("brightness-up", 0x06F),
    ("brightness-down", 0x070),
    ("config", 0x183),
    ("mail", 0x18A),
    ("calculator", 0x192),
    ("file-manager", 0x194),
    ("browser", 0x196),
    ("screen-lock", 0x19E),
    ("logoff", 0x19C),
    ("undo", 0x21A),
    ("copy", 0x21B),
    ("cut", 0x21C),
    ("paste", 0x21D),
    ("search", 0x221),
    ("home", 0x223),
    ("browser-back", 0x224),
    ("browser-forward", 0x225),
    ("browser-stop", 0x226),
    ("refresh", 0x227),
    ("bookmarks", 0x22A),
    ("zoom-in", 0x22D),
    ("zoom-out", 0x22E),
];

const CONSUMER_MAX: u16 = 0x2FFF;

pub fn button_slots() -> Vec<(&'static str, usize)> {
    BUTTONS.iter().map(|(n, s, _)| (*n, *s)).collect()
}

fn consumer_slot(name: &str) -> Result<Slot> {
    let code = match CONSUMER_CODES.iter().find(|(n, _)| *n == name) {
        Some(&(_, c)) => c,
        None => name
            .strip_prefix("0x")
            .and_then(|h| u16::from_str_radix(h, 16).ok())
            .ok_or_else(|| err!("unknown consumer key {name:?} (see `lindwurm actions`)"))?,
    };
    if code == 0 || code > CONSUMER_MAX {
        return Err(err!(
            "consumer code 0x{code:X} outside 0x1-0x{CONSUMER_MAX:X}"
        ));
    }
    Ok([0x04, 0x00, (code >> 8) as u8, code as u8])
}

fn raw_slot(hex: &str) -> Result<Slot> {
    if hex.len() != 8 {
        return Err(err!("raw slot {hex:?} needs 8 hex digits"));
    }
    let mut s = [0u8; 4];
    for (i, b) in s.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)
            .map_err(|_| err!("raw slot {hex:?} is not hex"))?;
    }
    Ok(s)
}

pub fn build_layout(c: &Config) -> Result<Layout> {
    for name in c.buttons.keys() {
        if !BUTTONS.iter().any(|(n, _, _)| n == name) {
            return Err(err!("unknown button {name:?}"));
        }
    }
    let mut l = Layout::default();
    let mut macro_index: HashMap<String, usize> = HashMap::new();
    for (name, slot, default) in BUTTONS {
        let own = c.buttons.get(*name).map(String::as_str);
        let action = canonical_action(own.unwrap_or(default));
        let s = resolve_action(action, c, &mut l, &mut macro_index)
            .map_err(|e| err!("button {name}: {e}"))?;
        l.slots[*slot] = s;
        if own.is_some() && action != *default && HOST_SIDE.contains(&action) {
            l.warnings.push(format!(
                "button {name}: {action} needs a host-side handler (the vendor's hid.exe); Linux ignores it"
            ));
        }
        if s[0] == 0 && s[1] != 0 && s[1] < 0xF0 {
            if let Some(w) = key_warning(s[1]) {
                l.warnings.push(format!("button {name}: {w}"));
            }
        }
    }
    let mut warnings = Vec::new();
    for (i, m) in l.macros.iter().enumerate() {
        for code in m.iter().skip(3).step_by(2) {
            if let Some(w) = key_warning(*code) {
                warnings.push(format!("macro {i}: {w}"));
            }
        }
    }
    l.warnings.extend(warnings);
    Ok(l)
}

fn resolve_action(
    action: &str,
    c: &Config,
    l: &mut Layout,
    macro_index: &mut HashMap<String, usize>,
) -> Result<Slot> {
    if let Some(&(_, s)) = ACTIONS.iter().find(|(n, _)| *n == action) {
        return Ok(s);
    }
    if let Some(name) = action.strip_prefix("consumer:") {
        return consumer_slot(name);
    }
    if let Some(hex) = action.strip_prefix("raw:") {
        return raw_slot(hex);
    }
    if let Some(k) = action.strip_prefix("key:") {
        let code = key_code(k).ok_or_else(|| err!("unknown key {k:?}"))?;
        return Ok([0x00, code, 0, 0]);
    }
    if let Some((name, steps)) = BUILTIN_MACROS.iter().find(|(n, _)| *n == action) {
        let key = format!("@{name}");
        let mc = MacroConfig {
            repeat: 1,
            steps: steps.iter().map(|s| s.to_string()).collect(),
        };
        return macro_slot(&key, &mc, l, macro_index);
    }
    if let Some(name) = action.strip_prefix("macro:") {
        let mc = c
            .macros
            .get(name)
            .ok_or_else(|| err!("macro {name:?} is not defined"))?;
        return macro_slot(name, mc, l, macro_index);
    }
    Err(err!("unknown action {action:?} (see `lindwurm actions`)"))
}

fn macro_slot(
    key: &str,
    mc: &MacroConfig,
    l: &mut Layout,
    macro_index: &mut HashMap<String, usize>,
) -> Result<Slot> {
    let idx = match macro_index.get(key) {
        Some(&i) => i,
        None => {
            let i = macro_index.len();
            if i >= NUM_MACROS {
                return Err(err!("more than {NUM_MACROS} macros in use"));
            }
            l.macros[i] = encode_macro(mc).map_err(|e| err!("macro {key}: {e}"))?;
            macro_index.insert(key.to_string(), i);
            i
        }
    };
    let steps = (l.macros[idx].len() - 2) / 2;
    Ok([0x53, 0x00, idx as u8, (2 * (steps + 1)) as u8])
}

struct Step {
    release: bool,
    key: u8,
    delay: u32, // units of 50 ms
}

/// Step syntax: "+k" press, "-k" release, "k" press and release, "150ms" pause
/// after the previous step. The device stores each pause with the step before it.
fn parse_steps(tokens: &[String]) -> Result<Vec<Step>> {
    let mut steps: Vec<Step> = Vec::new();
    let mut held: Vec<u8> = Vec::new();
    for t in tokens {
        if let Some(d) = t.strip_suffix("ms") {
            let ms: u32 = d
                .parse()
                .ok()
                .filter(|ms| ms % 50 == 0)
                .ok_or_else(|| err!("pause {t:?} must be a multiple of 50ms"))?;
            let last = steps
                .last_mut()
                .ok_or_else(|| err!("pause {t:?} needs a key step before it"))?;
            last.delay += ms / 50;
            if last.delay > 127 {
                return Err(err!("pause after one step is limited to 6350ms"));
            }
            continue;
        }
        let (kind, name) = match t.as_bytes().first() {
            None => return Err(err!("empty step")),
            Some(b'+') => ('+', &t[1..]),
            Some(b'-') => ('-', &t[1..]),
            _ => ('=', t.as_str()),
        };
        let key = key_code(name).ok_or_else(|| err!("unknown key {name:?}"))?;
        match kind {
            '+' => {
                if held.contains(&key) {
                    return Err(err!("{t:?}: key is already held"));
                }
                held.push(key);
                steps.push(Step {
                    release: false,
                    key,
                    delay: 0,
                });
            }
            '-' => {
                // Verified on the device: releasing a key that is not held
                // aborts the macro at that step.
                let pos = held
                    .iter()
                    .position(|&k| k == key)
                    .ok_or_else(|| err!("{t:?}: key is not held"))?;
                held.remove(pos);
                steps.push(Step {
                    release: true,
                    key,
                    delay: 0,
                });
            }
            _ => {
                if held.contains(&key) {
                    return Err(err!("{t:?}: key is already held"));
                }
                steps.push(Step {
                    release: false,
                    key,
                    delay: 0,
                });
                steps.push(Step {
                    release: true,
                    key,
                    delay: 0,
                });
            }
        }
    }
    if steps.is_empty() {
        return Err(err!("no steps"));
    }
    // The device does not release keys at the end of a macro, they stay stuck.
    if let Some(k) = held.first() {
        return Err(err!("key 0x{k:02X} is pressed but never released"));
    }
    if steps.len() > MAX_MACRO_STEPS {
        return Err(err!(
            "{} steps, the device holds at most {MAX_MACRO_STEPS}",
            steps.len()
        ));
    }
    Ok(steps)
}

/// Validates a macro and returns its number of device steps.
pub fn check_macro(mc: &MacroConfig) -> Result<usize> {
    Ok((encode_macro(mc)?.len() - 2) / 2)
}

fn encode_macro(mc: &MacroConfig) -> Result<Vec<u8>> {
    let repeat = if mc.repeat == 0 { 1 } else { mc.repeat };
    // Verified on the device up to 258; the meaning of the top bit is unknown.
    if repeat > 0x7FFF {
        return Err(err!("repeat must be between 1 and 32767"));
    }
    let mut data = vec![(repeat >> 8) as u8, repeat as u8];
    for s in parse_steps(&mc.steps)? {
        data.push(s.delay as u8 | if s.release { 0x80 } else { 0 });
        data.push(s.key);
    }
    Ok(data)
}

pub fn action_help() -> String {
    let mut s = String::new();
    for (n, _) in ACTIONS {
        let aliases: Vec<&str> = ACTION_ALIASES
            .iter()
            .filter(|(_, c)| c == n)
            .map(|(a, _)| *a)
            .collect();
        let host = if HOST_SIDE.contains(n) {
            "  (needs a host-side handler)"
        } else {
            ""
        };
        match aliases.is_empty() {
            true => s += &format!("{n}{host}\n"),
            false => s += &format!("{n}  (also: {}){host}\n", aliases.join(", ")),
        }
    }
    s += "built-in macros, as in the vendor software:\n";
    for (n, steps) in BUILTIN_MACROS {
        s += &format!("                  {n:<16} {}\n", steps.join(" "));
    }
    s += "key:<name>       single key, e.g. key:g, key:f5, key:0x0a (Linux drops codes above 0x65)\n";
    s += "macro:<name>     macro from the [macros] table\n";
    s += "consumer:<name>  consumer key, or consumer:0x<code> up to 0x2fff:\n";
    let mut names: Vec<_> = CONSUMER_CODES.to_vec();
    names.sort();
    for (n, c) in names {
        s += &format!("                  {n:<16} 0x{c:03X}\n");
    }
    s += "raw:<8 hex>      raw 4-byte slot, for experiments\n";
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(buttons: &[(&str, &str)], macros: &[(&str, &[&str])]) -> Config {
        Config {
            buttons: buttons
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            macros: macros
                .iter()
                .map(|(n, s)| {
                    let steps = s.iter().map(|x| x.to_string()).collect();
                    (n.to_string(), MacroConfig { repeat: 0, steps })
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn macro_encoding() {
        let c = cfg(
            &[("side-right", "macro:hi"), ("thumb-bottom", "key:g")],
            &[("hi", &["+h", "50ms", "-h", "50ms", "i", "50ms"])],
        );
        let l = build_layout(&c).unwrap();
        assert_eq!(
            l.macros[0],
            [0x00, 0x01, 0x01, 0x0B, 0x81, 0x0B, 0x00, 0x0C, 0x81, 0x0C]
        );
        assert_eq!(l.slots[9], [0x53, 0x00, 0x00, 0x0A]);
        assert_eq!(l.slots[10], [0x00, 0x0A, 0, 0]);
        assert_eq!(l.slots[0], [0x00, 0xF0, 0, 0]);
    }

    #[test]
    fn max_steps() {
        let mut steps = vec!["+lshift"];
        let letters: Vec<String> = ('a'..='v').map(|c| c.to_string()).collect();
        steps.extend(letters.iter().map(String::as_str));
        steps.push("-lshift");
        let c = cfg(&[("side-right", "macro:m")], &[("m", &steps)]);
        assert_eq!(build_layout(&c).unwrap().macros[0].len(), 2 + 46 * 2);
        steps.insert(1, "w");
        let c = cfg(&[("side-right", "macro:m")], &[("m", &steps)]);
        assert!(build_layout(&c).is_err());
    }

    #[test]
    fn consumer_and_warnings() {
        let c = cfg(
            &[
                ("side-right", "consumer:calculator"),
                ("thumb-bottom", "consumer:0x224"),
                ("tilt-left", "key:f13"),
                ("tilt-right", "key:f5"),
            ],
            &[],
        );
        let l = build_layout(&c).unwrap();
        assert_eq!(l.slots[9], [0x04, 0x00, 0x01, 0x92]);
        assert_eq!(l.slots[10], [0x04, 0x00, 0x02, 0x24]);
        assert_eq!(l.warnings.len(), 1, "{:?}", l.warnings);
    }

    #[test]
    fn errors() {
        let bad: Vec<Config> = vec![
            cfg(&[("nope", "none")], &[]),
            cfg(&[("left", "fly")], &[]),
            cfg(&[("left", "macro:missing")], &[]),
            cfg(&[("left", "macro:m")], &[("m", &["30ms"])]),
            cfg(&[("left", "macro:m")], &[("m", &["a", "75ms"])]),
            cfg(&[("left", "macro:m")], &[("m", &["-z", "a"])]),
            cfg(&[("left", "macro:m")], &[("m", &["a", "+f24"])]),
            cfg(&[("left", "macro:m")], &[("m", &[""])]),
            cfg(&[("left", "consumer:0x3000")], &[]),
            cfg(&[("left", "consumer:nope")], &[]),
            cfg(&[("left", "consumer:0x0")], &[]),
            cfg(&[("left", "raw:0400")], &[]),
        ];
        for (i, c) in bad.iter().enumerate() {
            assert!(build_layout(c).is_err(), "case {i} should fail");
        }
        assert!(Config::parse("dpi = 9").is_err());
        assert!(Config::parse("unknown = 1").is_err());
    }
}
