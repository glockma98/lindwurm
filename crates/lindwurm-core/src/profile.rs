use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::{Config, build_layout, button_slots};
use crate::protocol::{Layout, describe_layout};
use crate::{Result, err};

pub const DEFAULT_PROFILE: &str = "default";
pub const FACTORY_PROFILE: &str = "factory";
/// Factory settings, available as profile "factory" unless a file of that
/// name overrides it.
pub const FACTORY_TOML: &str = include_str!("factory.toml");

/// Profiles live in `<config dir>/lindwurm/profiles`; the last write is
/// recorded in `<state dir>/lindwurm/applied` because the mouse cannot be
/// read back. XDG variables are honoured on Linux.
#[derive(Debug, Clone)]
pub struct Store {
    pub profiles_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl Store {
    pub fn new() -> Result<Store> {
        let dirs = directories::ProjectDirs::from("", "", "lindwurm")
            .ok_or_else(|| err!("no home directory"))?;
        let state = dirs
            .state_dir()
            .unwrap_or_else(|| dirs.data_local_dir())
            .to_path_buf();
        Ok(Store {
            profiles_dir: dirs.config_dir().join("profiles"),
            state_dir: state,
        })
    }

    pub fn at(root: &Path) -> Store {
        Store {
            profiles_dir: root.join("profiles"),
            state_dir: root.join("state"),
        }
    }

    /// A bare name refers to a file in the profiles directory, anything else is a path.
    pub fn resolve(&self, arg: &str) -> Result<(PathBuf, String)> {
        if is_name(arg) {
            return Ok((
                self.profiles_dir.join(format!("{arg}.toml")),
                arg.to_string(),
            ));
        }
        let p = PathBuf::from(arg);
        let toml_ext = p
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
        if toml_ext || p.components().count() > 1 {
            let name = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            return Ok((p, name));
        }
        Err(err!(
            "invalid profile name {arg:?} (letters, digits, - and _)"
        ))
    }

    pub fn load(&self, arg: &str) -> Result<(Config, String)> {
        let (path, name) = self.resolve(arg)?;
        let c = self.load_chain(&path, &mut HashSet::new())?;
        Ok((c, name))
    }

    /// Each profile of the extends chain with its own settings, starting with
    /// the profile itself.
    pub fn chain(&self, arg: &str) -> Result<Vec<(String, Config)>> {
        let (mut path, mut name) = self.resolve(arg)?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        loop {
            let key = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen.insert(key) {
                return Err(err!("{}: extends loop", path.display()));
            }
            let c = Config::parse(&self.read_profile(&path)?)
                .map_err(|e| err!("{}: {e}", path.display()))?;
            let parent = c.extends.clone();
            out.push((name, c));
            let Some(parent) = parent else {
                return Ok(out);
            };
            let (mut p, n) = self.resolve(&parent)?;
            if !is_name(&parent) && p.is_relative() {
                p = path.parent().unwrap_or(Path::new(".")).join(p);
            }
            (path, name) = (p, n);
        }
    }

    fn load_chain(&self, path: &Path, seen: &mut HashSet<PathBuf>) -> Result<Config> {
        let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !seen.insert(key) {
            return Err(err!("{}: extends loop", path.display()));
        }
        let text = self.read_profile(path)?;
        let c = Config::parse(&text).map_err(|e| err!("{}: {e}", path.display()))?;
        let Some(parent) = &c.extends else {
            return Ok(c);
        };
        let (mut parent_path, _) = self.resolve(parent)?;
        if !is_name(parent) && parent_path.is_relative() {
            parent_path = path.parent().unwrap_or(Path::new(".")).join(parent_path);
        }
        let p = self.load_chain(&parent_path, seen)?;
        Ok(merge(p, c))
    }

    fn is_builtin_factory(&self, path: &Path) -> bool {
        !path.exists() && path == self.profiles_dir.join(format!("{FACTORY_PROFILE}.toml"))
    }

    /// Text of a profile file, or of the built-in factory profile.
    pub fn read_profile(&self, path: &Path) -> Result<String> {
        if self.is_builtin_factory(path) {
            return Ok(FACTORY_TOML.to_string());
        }
        fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => err!("profile {} not found", path.display()),
            _ => err!("{}: {e}", path.display()),
        })
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.resolve(name)
            .is_ok_and(|(p, _)| self.is_builtin_factory(&p))
    }

    /// "default" if such a profile exists, otherwise the factory settings.
    pub fn default_profile(&self) -> String {
        match self.resolve(DEFAULT_PROFILE) {
            Ok((p, _)) if p.exists() => DEFAULT_PROFILE.to_string(),
            _ => FACTORY_PROFILE.to_string(),
        }
    }

    pub fn list(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let entries = match fs::read_dir(&self.profiles_dir) {
            Ok(e) => e.collect::<Vec<_>>(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        names.push(FACTORY_PROFILE.to_string());
        for e in entries {
            let p = e?.path();
            if p.extension().is_some_and(|x| x == "toml")
                && let Some(stem) = p.file_stem()
            {
                names.push(stem.to_string_lossy().into_owned());
            }
        }
        names.sort();
        names.dedup();
        Ok(names)
    }

    pub fn write_state(&self, name: &str, c: &Config, l: &Layout) -> Result<()> {
        fs::create_dir_all(&self.state_dir)?;
        let secs = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let s = format!("profile {name}\napplied {secs}\n{}", state_body(c, l));
        fs::write(self.state_dir.join("applied"), s)?;
        Ok(())
    }

    pub fn read_state(&self) -> Result<Option<String>> {
        match fs::read_to_string(self.state_dir.join("applied")) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Whether buttons and macros equal the last write from this computer.
    pub fn layout_unchanged(&self, l: &Layout) -> bool {
        let Ok(Some(state)) = self.read_state() else {
            return false;
        };
        let table = |s: &str| -> Vec<String> {
            s.lines()
                .filter(|l| l.starts_with("slot ") || l.starts_with("macro "))
                .map(str::to_string)
                .collect()
        };
        table(&state) == table(&describe_layout(l, &button_slots()))
    }

    /// Polling rate set by the last write, if any.
    pub fn applied_polling_rate(&self) -> Option<u16> {
        let s = self.read_state().ok()??;
        s.lines()
            .find_map(|l| l.strip_prefix("polling-rate ")?.trim().parse().ok())
    }

    /// Notes a polling rate set outside a profile write, so the next write
    /// does not wrongly skip it.
    pub fn record_polling_rate(&self, hz: u16) -> Result<()> {
        let Some(s) = self.read_state()? else {
            return Ok(());
        };
        let mut lines: Vec<String> = s
            .lines()
            .filter(|l| !l.starts_with("polling-rate "))
            .map(str::to_string)
            .collect();
        let at = lines
            .iter()
            .position(|l| l.starts_with("slot "))
            .unwrap_or(lines.len());
        lines.insert(at, format!("polling-rate {hz}"));
        fs::write(self.state_dir.join("applied"), lines.join("\n") + "\n")?;
        Ok(())
    }

    pub fn active(&self) -> Option<String> {
        let s = self.read_state().ok()??;
        s.lines()
            .next()?
            .strip_prefix("profile ")
            .map(str::to_string)
    }

    /// For the profile written last: whether its file now yields different
    /// bytes than were written. None for any other profile.
    pub fn changed_since_applied(&self, p: &Prepared) -> Option<bool> {
        let state = self.read_state().ok()??;
        let mut lines = state.lines();
        if lines.next()?.strip_prefix("profile ")? != p.name {
            return None;
        }
        let written: Vec<&str> = lines.skip(1).collect();
        let now = state_body(&p.config, &p.layout);
        Some(written != now.lines().collect::<Vec<_>>())
    }

    /// Loads and validates a profile without touching the device.
    pub fn prepare(&self, arg: &str) -> Result<Prepared> {
        let (config, name) = self.load(arg)?;
        let layout = build_layout(&config)?;
        Ok(Prepared {
            name,
            config,
            layout,
        })
    }
}

pub struct Prepared {
    pub name: String,
    pub config: Config,
    pub layout: Layout,
}

fn state_body(c: &Config, l: &Layout) -> String {
    let mut s = String::new();
    if let Some(d) = c.dpi {
        s += &format!("dpi     {d}\n");
    }
    for (k, v) in [
        ("color", &c.led.color),
        ("brightness", &c.led.brightness),
        ("effect", &c.led.effect),
    ] {
        if let Some(v) = v {
            s += &format!("led     {k} {v}\n");
        }
    }
    if let Some(p) = c.polling_rate {
        s += &format!("polling-rate {p}\n");
    }
    for (k, v) in [("x", c.sensitivity.x), ("y", c.sensitivity.y)] {
        if let Some(v) = v {
            s += &format!("sensitivity {k} {v}\n");
        }
    }
    s + &describe_layout(l, &button_slots())
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn merge(parent: Config, child: Config) -> Config {
    let mut m = parent;
    m.extends = None;
    if child.dpi.is_some() {
        m.dpi = child.dpi;
    }
    m.led.color = child.led.color.or(m.led.color);
    m.led.brightness = child.led.brightness.or(m.led.brightness);
    m.led.effect = child.led.effect.or(m.led.effect);
    m.polling_rate = child.polling_rate.or(m.polling_rate);
    m.sensitivity.x = child.sensitivity.x.or(m.sensitivity.x);
    m.sensitivity.y = child.sensitivity.y.or(m.sensitivity.y);
    m.buttons.extend(child.buttons);
    m.macros.extend(child.macros);
    m
}

/// Seconds since the epoch from the state file, formatted as UTC.
pub fn format_applied(secs: u64) -> String {
    let days = secs / 86400;
    let (h, mi, s) = (secs % 86400 / 3600, secs % 3600 / 60, secs % 60);
    // Civil-from-days (Howard Hinnant), valid for the Unix era.
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extends_state_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        fs::create_dir_all(&store.profiles_dir).unwrap();
        let write = |n: &str, body: &str| {
            fs::write(store.profiles_dir.join(format!("{n}.toml")), body).unwrap()
        };
        write(
            "default",
            "dpi = 3\n[buttons]\nthumb-bottom = \"key:g\"\nside-right = \"macro:hi\"\n[macros.hi]\nsteps = [\"h\", \"i\"]\n",
        );
        write(
            "fps",
            "extends = \"default\"\ndpi = 5\n[buttons]\nside-right = \"key:r\"\n",
        );
        write("loop-a", "extends = \"loop-b\"\n");
        write("loop-b", "extends = \"loop-a\"\n");

        let p = store.prepare("fps").unwrap();
        assert_eq!(p.name, "fps");
        assert_eq!(p.config.dpi, Some(5));
        assert_eq!(p.config.buttons["thumb-bottom"], "key:g");
        assert_eq!(p.layout.slots[9], [0x00, 0x15, 0, 0]);
        store.write_state(&p.name, &p.config, &p.layout).unwrap();
        assert_eq!(store.active().as_deref(), Some("fps"));
        assert_eq!(store.changed_since_applied(&p), Some(false));
        write("fps", "extends = \"default\"\ndpi = 6\n");
        let p2 = store.prepare("fps").unwrap();
        assert_eq!(store.changed_since_applied(&p2), Some(true));
        assert_eq!(
            store.changed_since_applied(&store.prepare("default").unwrap()),
            None
        );
        assert_eq!(
            store.list().unwrap().len(),
            5,
            "4 files plus the built-in factory"
        );
        assert!(store.load("loop-a").is_err());
        assert!(store.load("missing").is_err());
        assert!(store.resolve("bad name").is_err());
    }

    #[test]
    fn layout_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        fs::create_dir_all(&store.profiles_dir).unwrap();
        let write = |n: &str, body: &str| {
            fs::write(store.profiles_dir.join(format!("{n}.toml")), body).unwrap()
        };
        write(
            "a",
            "dpi = 2\n[buttons]\nside-right = \"macro:m\"\n[macros.m]\nsteps = [\"x\"]\n",
        );
        write(
            "b",
            "extends = \"a\"\ndpi = 6\n[led]\ncolor = \"#ff0000\"\n",
        );
        write("c", "extends = \"a\"\n[macros.m]\nsteps = [\"y\"]\n");
        let a = store.prepare("a").unwrap();
        assert!(!store.layout_unchanged(&a.layout), "nothing written yet");
        store.write_state(&a.name, &a.config, &a.layout).unwrap();
        assert!(store.layout_unchanged(&store.prepare("b").unwrap().layout));
        assert!(!store.layout_unchanged(&store.prepare("c").unwrap().layout));
    }

    #[test]
    fn applied_format() {
        assert_eq!(format_applied(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_applied(1_795_259_907), "2026-11-21 11:18:27 UTC");
    }
}
