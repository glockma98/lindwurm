//! Configuration of gaming mice built on the Holtek 04d9:a067 controller,
//! such as the Sharkoon Drakonia Black and the Perixx MX-2000/MX-3000.

pub mod config;
pub mod device;
pub mod edit;
pub mod ini;
pub mod keys;
pub mod profile;
pub mod protocol;

use std::fmt;

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<hidapi::HidError> for Error {
    fn from(e: hidapi::HidError) -> Self {
        Error(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Loads a profile, writes it to the mouse and records it as applied.
///
/// Writing buttons and macros takes most of the time, and a new polling rate
/// makes the mouse reconnect. With `quick` both are skipped when they match
/// the last write from this computer; that record is wrong if the mouse was
/// written elsewhere since, so it is opt-in. The returned flag tells whether
/// buttons and macros were skipped.
pub fn apply_profile(
    store: &profile::Store,
    arg: &str,
    quick: bool,
) -> Result<(profile::Prepared, bool)> {
    let p = store.prepare(arg)?;
    let dev = device::Device::open()?;
    let skip = quick && store.layout_unchanged(&p.layout);
    if skip {
        dev.set_normal_mode()?;
    } else {
        dev.write_layout(&p.layout)?;
    }
    if let Some(dpi) = p.config.dpi {
        dev.set_dpi(dpi)?;
    }
    dev.set_led(&p.config.led)?;
    let s = &p.config.sensitivity;
    if s.x.is_some() || s.y.is_some() {
        let max = config::SENSITIVITY_MAX;
        dev.set_sensitivity(s.x.unwrap_or(max), s.y.unwrap_or(max))?;
    }
    let rate = p
        .config
        .polling_rate
        .filter(|r| !quick || store.applied_polling_rate() != Some(*r));
    store.write_state(&p.name, &p.config, &p.layout)?;
    if let Some(r) = rate {
        dev.set_polling_rate(r)?;
    }
    Ok((p, skip))
}

/// The state file with its timestamp made readable.
pub fn status_text(store: &profile::Store) -> Result<Option<String>> {
    Ok(store.read_state()?.map(|s| {
        s.lines()
            .map(
                |l| match l.strip_prefix("applied ").and_then(|t| t.parse().ok()) {
                    Some(secs) => format!("applied {}", profile::format_applied(secs)),
                    None => l.to_string(),
                },
            )
            .collect::<Vec<_>>()
            .join("\n")
    }))
}

macro_rules! err {
    ($($arg:tt)*) => { $crate::Error(format!($($arg)*)) };
}
pub(crate) use err;
