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
pub fn apply_profile(store: &profile::Store, arg: &str) -> Result<profile::Prepared> {
    let p = store.prepare(arg)?;
    let dev = device::Device::open()?;
    dev.write_layout(&p.layout)?;
    if let Some(dpi) = p.config.dpi {
        dev.set_dpi(dpi)?;
    }
    dev.set_led(&p.config.led)?;
    store.write_state(&p.name, &p.config, &p.layout)?;
    Ok(p)
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
