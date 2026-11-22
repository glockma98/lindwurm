use std::thread::sleep;
use std::time::Duration;

use hidapi::{HidApi, HidDevice};

use crate::config::{
    check_sensitivity, led_level, parse_color, polling_code, LedConfig, LED_BRIGHTNESS, LED_EFFECT,
};
use crate::protocol::{scrambled, Layout, FEATURE_REPORT, MACRO_BYTES, NUM_SLOTS};
use crate::{err, Result};

pub const VENDOR_ID: u16 = 0x04D9;
pub const PRODUCT_ID: u16 = 0xA067;
// The reports go out synchronously; the pause is only a safety margin. A
// Drakonia Black took a full write without any pause (0.76 s), 2 ms keeps it
// at about 1.2 s instead of 2.8 s with the former 10 ms.
const REPORT_INTERVAL: Duration = Duration::from_millis(2);

/// Same text as contrib/70-lindwurm.rules (checked by a test).
pub const UDEV_RULE: &str =
    "# Lets the logged-in user configure mice with the Holtek 04d9:a067 controller
# (e.g. Sharkoon Drakonia Black) through hidraw. The file name must sort before
# 73-seat-late.rules, which turns the uaccess tag into an ACL.
KERNEL==\"hidraw*\", ATTRS{idVendor}==\"04d9\", ATTRS{idProduct}==\"a067\", TAG+=\"uaccess\"
";

/// hidapi only reports a generic open failure; on Linux check the node
/// first so a missing udev rule gets a useful message.
fn check_access(info: &hidapi::DeviceInfo) -> Result<()> {
    if cfg!(target_os = "linux") {
        if let Ok(path) = info.path().to_str() {
            if let Err(e) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    return Err(err!(
                        "no permission to open {path}; install the udev rule (`lindwurm udev-rule`) and replug the mouse"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Interface 1 carries feature report 7 (vendor page 0xFF01), interface 2
/// takes the 8-byte output reports without report ID. Windows and macOS
/// split interface 1 into one device per top-level collection, Linux hidraw
/// exposes it as a single node.
pub struct Device {
    ctl: HidDevice,
    out: HidDevice,
}

impl Device {
    pub fn open() -> Result<Device> {
        let api = HidApi::new()?;
        let mut ctl = None;
        let mut out = None;
        for info in api.device_list() {
            if info.vendor_id() != VENDOR_ID || info.product_id() != PRODUCT_ID {
                continue;
            }
            match info.interface_number() {
                1 if cfg!(target_os = "linux") || info.usage_page() == 0xFF01 => {
                    ctl.get_or_insert(info)
                }
                2 => out.get_or_insert(info),
                _ => continue,
            };
        }
        let ctl = ctl.ok_or_else(|| err!("no supported mouse (04d9:a067, e.g. Sharkoon Drakonia) found: control interface missing"))?;
        let out = out.ok_or_else(|| err!("no supported mouse (04d9:a067, e.g. Sharkoon Drakonia) found: data interface missing"))?;
        check_access(ctl)?;
        check_access(out)?;
        Ok(Device {
            ctl: ctl.open_device(&api)?,
            out: out.open_device(&api)?,
        })
    }

    pub fn command(&self, cmd: u8, a: u8, b: u8, c: u8) -> Result<()> {
        let p = scrambled([cmd, a, b, c, 0, 0, 0]);
        let mut buf = [0u8; 8];
        buf[0] = FEATURE_REPORT;
        buf[1..].copy_from_slice(&p);
        self.ctl
            .send_feature_report(&buf)
            .map_err(|e| err!("feature command {cmd:02X}: {e}"))?;
        sleep(REPORT_INTERVAL);
        Ok(())
    }

    fn output(&self, data: &[u8]) -> Result<()> {
        let mut plain = [0u8; 8];
        plain[..data.len()].copy_from_slice(data);
        let mut tail = [0u8; 7];
        tail.copy_from_slice(&plain[1..]);
        let mut buf = [0u8; 9];
        buf[1] = plain[0];
        buf[2..].copy_from_slice(&scrambled(tail));
        self.out.write(&buf)?;
        sleep(REPORT_INTERVAL);
        Ok(())
    }

    pub fn set_dpi(&self, level: u8) -> Result<()> {
        if !(1..=7).contains(&level) {
            return Err(err!("dpi needs a level from 1 to 7"));
        }
        self.command(0x09, level, 0, 0)
    }

    /// The mouse disconnects and reconnects with the new USB polling
    /// interval right after this command, so it must be the last one sent.
    pub fn set_polling_rate(&self, hz: u16) -> Result<()> {
        self.command(0x01, polling_code(hz)?, 0, 0)
    }

    pub fn set_sensitivity(&self, x: u8, y: u8) -> Result<()> {
        self.command(0x12, check_sensitivity(y)?, check_sensitivity(x)?, 0)
    }

    /// Sends only the LED settings the profile sets. Order and commands as in
    /// the vendor software: color 0A (G, R, B), effect 0B, brightness 0C.
    pub fn set_led(&self, led: &LedConfig) -> Result<()> {
        if let Some(c) = &led.color {
            let [r, g, b] = parse_color(c)?;
            self.command(0x0A, g, r, b)?;
        }
        if let Some(e) = &led.effect {
            self.command(0x0B, led_level(LED_EFFECT, e)?, 0, 0)?;
        }
        if let Some(b) = &led.brightness {
            self.command(0x0C, led_level(LED_BRIGHTNESS, b)?, 0, 0)?;
        }
        Ok(())
    }

    /// Replays the vendor software's sequence: begin (03), commit (02),
    /// button table length (04), 8 table reports, macro block length (05,
    /// 0x0600 bytes), 16 macros of 12 reports each.
    pub fn write_layout(&self, l: &Layout) -> Result<()> {
        self.command(0x03, 1, 0, 0)?;
        self.command(0x02, 1, 0, 0)?;
        self.command(0x04, 1, (NUM_SLOTS * 4) as u8, 0)?;
        for pair in l.slots.chunks(2) {
            let mut p = [0u8; 8];
            p[..4].copy_from_slice(&pair[0]);
            p[4..].copy_from_slice(&pair[1]);
            self.output(&p)?;
        }
        self.command(0x05, 1, 0, 6)?;
        for m in &l.macros {
            let mut flat = [0u8; MACRO_BYTES];
            flat[..m.len()].copy_from_slice(m);
            for chunk in flat.chunks(8) {
                self.output(chunk)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn udev_rule_matches_contrib_file() {
        let file = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contrib/70-lindwurm.rules"
        );
        if let Ok(text) = std::fs::read_to_string(file) {
            // A checkout with CRLF line endings must not fail the comparison.
            assert_eq!(text.replace("\r\n", "\n"), super::UDEV_RULE);
        }
    }
}
