//! Field-level view and editing of profiles, shared by the TUI and GUI.
//! Edits go through toml_edit so comments and layout of the file survive.

use std::fs;

use toml_edit::{value, Array, DocumentMut, Item, Key, Table};

use crate::config::{canonical_button, Config, MacroConfig, BUTTONS, BUTTON_ALIASES};
use crate::profile::Store;
use crate::{err, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// Set in the profile's own file.
    Own,
    /// Comes from a profile named in `extends`.
    Inherited,
    /// Built-in default, or nothing is sent.
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// "extends", "dpi", "polling-rate", "sensitivity.x", "sensitivity.y", "led.color",
    /// "led.brightness", "led.effect", "buttons.<name>" or "macros.<name>".
    pub key: String,
    pub value: String,
    pub source: Source,
    /// For inherited values, the profile in the extends chain that sets it.
    pub from: Option<String>,
}

/// Macro value as one string: optional "repeat=N" followed by the steps.
pub fn macro_value(m: &MacroConfig) -> String {
    let steps = m.steps.join(" ");
    if m.repeat > 1 {
        format!("repeat={} {steps}", m.repeat)
    } else {
        steps
    }
}

pub fn parse_macro_value(s: &str) -> Result<MacroConfig> {
    let mut tokens = s.split_whitespace().peekable();
    let mut repeat = 0;
    if let Some(r) = tokens.peek().and_then(|t| t.strip_prefix("repeat=")) {
        repeat = r.parse().map_err(|_| err!("repeat={r} is not a number"))?;
        tokens.next();
    }
    let steps: Vec<String> = tokens.map(str::to_string).collect();
    if steps.is_empty() {
        return Err(err!("a macro needs at least one step"));
    }
    Ok(MacroConfig { repeat, steps })
}

fn entries(c: &Config) -> Vec<(String, String)> {
    let mut v = Vec::new();
    if let Some(e) = &c.extends {
        v.push(("extends".into(), e.clone()));
    }
    if let Some(d) = c.dpi {
        v.push(("dpi".into(), d.to_string()));
    }
    for (k, x) in [
        ("led.color", &c.led.color),
        ("led.brightness", &c.led.brightness),
        ("led.effect", &c.led.effect),
    ] {
        if let Some(x) = x {
            v.push((k.into(), x.clone()));
        }
    }
    if let Some(p) = c.polling_rate {
        v.push(("polling-rate".into(), p.to_string()));
    }
    for (k, x) in [
        ("sensitivity.x", c.sensitivity.x),
        ("sensitivity.y", c.sensitivity.y),
    ] {
        if let Some(x) = x {
            v.push((k.into(), x.to_string()));
        }
    }
    for (b, a) in &c.buttons {
        v.push((format!("buttons.{b}"), a.clone()));
    }
    for (n, m) in &c.macros {
        v.push((format!("macros.{n}"), macro_value(m)));
    }
    v
}

impl Store {
    /// Every editable field of a profile with its effective value.
    pub fn fields(&self, arg: &str) -> Result<Vec<Field>> {
        let chain: Vec<(String, Vec<(String, String)>)> = self
            .chain(arg)?
            .iter()
            .map(|(n, c)| (n.clone(), entries(c)))
            .collect();
        let lookup = |k: &str| -> (String, Source, Option<String>) {
            // A parent's own extends is not a setting of this profile.
            let levels = if k == "extends" { 1 } else { chain.len() };
            for (i, (name, e)) in chain.iter().enumerate().take(levels) {
                if let Some((_, v)) = e.iter().find(|(x, _)| x == k) {
                    return match i {
                        0 => (v.clone(), Source::Own, None),
                        _ => (v.clone(), Source::Inherited, Some(name.clone())),
                    };
                }
            }
            (String::new(), Source::Default, None)
        };
        let mut keys: Vec<String> = [
            "extends",
            "dpi",
            "polling-rate",
            "sensitivity.x",
            "sensitivity.y",
            "led.color",
            "led.brightness",
            "led.effect",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        keys.extend(BUTTONS.iter().map(|(n, _, _)| format!("buttons.{n}")));
        keys.extend(
            chain
                .iter()
                .rev()
                .flat_map(|(_, e)| e.iter())
                .filter(|(k, _)| k.starts_with("macros."))
                .map(|(k, _)| k.clone())
                .collect::<std::collections::BTreeSet<_>>(),
        );
        Ok(keys
            .into_iter()
            .map(|key| {
                let (mut value, source, from) = lookup(&key);
                if source == Source::Default {
                    if let Some(b) = key.strip_prefix("buttons.") {
                        value = BUTTONS
                            .iter()
                            .find(|(n, _, _)| *n == b)
                            .map(|(_, _, d)| d.to_string())
                            .unwrap_or_default();
                    }
                }
                Field {
                    key,
                    value,
                    source,
                    from,
                }
            })
            .collect())
    }

    /// Sets a field in the profile's own file, or removes it with `None` so the
    /// inherited or default value applies again. The profile is validated
    /// before the change is kept.
    pub fn set_field(&self, arg: &str, key: &str, new: Option<&str>) -> Result<()> {
        let (path, name) = self.resolve(arg)?;
        if self.is_builtin(&name) {
            return Err(err!("{name} is built in; clone it to change it"));
        }
        let old = fs::read_to_string(&path)?;
        let mut doc: DocumentMut = old.parse().map_err(|e| err!("{}: {e}", path.display()))?;
        edit_doc(&mut doc, key, new)?;
        fs::write(&path, doc.to_string())?;
        if let Err(e) = self.prepare(arg) {
            fs::write(&path, old)?;
            return Err(e);
        }
        Ok(())
    }

    pub fn clone_profile(&self, from: &str, to: &str) -> Result<()> {
        let (src, _) = self.resolve(from)?;
        let text = self.read_profile(&src)?;
        let (dst, name) = self.resolve(to)?;
        if name != to {
            return Err(err!("new profile needs a plain name"));
        }
        if dst.exists() {
            return Err(err!("profile {to} already exists"));
        }
        fs::create_dir_all(&self.profiles_dir)?;
        fs::write(dst, text)?;
        Ok(())
    }

    /// Writes the profiles of a vendor Mouse.ini as <prefix>-1 .. <prefix>-5.
    /// Nothing is written if any of the names already exists.
    pub fn import_ini(
        &self,
        text: &str,
        source: &str,
        prefix: &str,
    ) -> Result<Vec<(String, crate::ini::Imported)>> {
        let imported = crate::ini::import(text, source)?;
        let mut named = Vec::new();
        for i in imported {
            let name = format!("{prefix}-{}", i.group + 1);
            let (path, n) = self.resolve(&name)?;
            if n != name {
                return Err(err!("invalid prefix {prefix:?}"));
            }
            if path.exists() {
                return Err(err!("profile {name} already exists, choose another prefix"));
            }
            named.push((name, path, i));
        }
        fs::create_dir_all(&self.profiles_dir)?;
        Ok(named
            .into_iter()
            .map(|(name, path, i)| fs::write(&path, &i.toml).map(|_| (name, i)))
            .collect::<std::io::Result<Vec<_>>>()?)
    }

    /// Reads a Mouse.ini from disk and imports it, see `import_ini`.
    pub fn import_ini_file(
        &self,
        path: &std::path::Path,
        prefix: &str,
    ) -> Result<Vec<(String, crate::ini::Imported)>> {
        let bytes = fs::read(path).map_err(|e| err!("{}: {e}", path.display()))?;
        // The vendor file is plain ASCII; be lenient with anything else.
        let text = String::from_utf8_lossy(&bytes);
        let source = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        self.import_ini(&text, &source, prefix)
    }

    /// One-line summary of an import for status lines.
    pub fn import_summary(result: &[(String, crate::ini::Imported)]) -> String {
        let names: Vec<&str> = result.iter().map(|(n, _)| n.as_str()).collect();
        let warnings: usize = result.iter().map(|(_, i)| i.warnings.len()).sum();
        match warnings {
            0 => format!("imported {}", names.join(", ")),
            w => format!(
                "imported {} with {w} warning(s), see the comments in the profiles",
                names.join(", ")
            ),
        }
    }

    pub fn delete_profile(&self, name: &str) -> Result<()> {
        let (path, _) = self.resolve(name)?;
        if self.is_builtin(name) {
            return Err(err!("{name} is built in"));
        }
        for other in self.list()? {
            if other == name {
                continue;
            }
            if let Ok((c, _)) = self
                .resolve(&other)
                .and_then(|(p, _)| Ok((Config::parse(&self.read_profile(&p)?)?, ())))
            {
                if c.extends.as_deref() == Some(name) {
                    return Err(err!("profile {other} extends {name}"));
                }
            }
        }
        fs::remove_file(path)?;
        Ok(())
    }
}

fn decor_prefix(t: &Table, key: &str) -> Option<String> {
    let decor = match t.get(key) {
        Some(Item::Table(sub)) => sub.decor(),
        _ => t.key(key)?.leaf_decor(),
    };
    decor.prefix()?.as_str().map(str::to_string)
}

fn set_prefix(t: &mut Table, key: &str, prefix: &str) {
    if let Some(Item::Table(sub)) = t.get_mut(key) {
        sub.decor_mut().set_prefix(prefix);
    } else if let Some(mut k) = t.key_mut(key) {
        k.leaf_decor_mut().set_prefix(prefix);
    }
}

// toml_edit stores a comment as part of the line below it, so removing that
// line would drop e.g. a file header. Hand such comments to the next entry.
fn remove_keeping_comments(t: &mut Table, key: &str) {
    let prefix = decor_prefix(t, key).filter(|p| p.contains('#'));
    t.remove(key);
    let Some(p) = prefix else { return };
    if let Some((mut k, item)) = t.iter_mut().next() {
        let decor = match item.as_table_mut() {
            Some(sub) => sub.decor_mut(),
            None => k.leaf_decor_mut(),
        };
        let old = decor
            .prefix()
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        decor.set_prefix(format!("{p}{old}"));
    }
}

fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    if !doc.contains_table(name) {
        doc[name] = Item::Table(Table::new());
    }
    doc[name].as_table_mut().expect("table was just ensured")
}

fn edit_doc(doc: &mut DocumentMut, key: &str, new: Option<&str>) -> Result<()> {
    let new = new.map(str::trim).filter(|s| !s.is_empty());
    match key.split_once('.') {
        None if key == "dpi" => match new {
            Some(v) => {
                let d: i64 = v
                    .parse()
                    .map_err(|_| err!("dpi must be a level from 1 to 7"))?;
                doc["dpi"] = value(d);
            }
            None => remove_keeping_comments(doc.as_table_mut(), "dpi"),
        },
        None if key == "extends" => match new {
            Some(v) => {
                let root = doc.as_table_mut();
                if let Some(item) = root.get_mut("extends") {
                    *item = value(v);
                } else {
                    // A new extends goes first, below the file's header comment.
                    let first = root.iter().next().map(|(k, _)| k.to_string());
                    let header = first
                        .as_deref()
                        .and_then(|k| decor_prefix(root, k))
                        .filter(|p| p.contains('#'));
                    let rest: Vec<(Key, Item)> = root
                        .iter()
                        .map(|(k, i)| (root.key(k).cloned().expect("key exists"), i.clone()))
                        .collect();
                    root.clear();
                    root.insert("extends", value(v));
                    for (k, i) in rest {
                        root.insert_formatted(&k, i);
                    }
                    if let (Some(h), Some(first)) = (header, first) {
                        set_prefix(root, "extends", &h);
                        set_prefix(root, &first, "\n");
                    }
                }
            }
            None => remove_keeping_comments(doc.as_table_mut(), "extends"),
        },
        None if key == "polling-rate" => match new {
            Some(v) => {
                let hz: u16 = v
                    .parse()
                    .map_err(|_| err!("polling rate must be a number in Hz"))?;
                doc["polling-rate"] = value(i64::from(hz));
            }
            None => remove_keeping_comments(doc.as_table_mut(), "polling-rate"),
        },
        Some(("sensitivity", axis @ ("x" | "y"))) => {
            let t = table(doc, "sensitivity");
            match new {
                Some(v) => {
                    let s: u8 = v
                        .parse()
                        .map_err(|_| err!("sensitivity must be from 1 to 10"))?;
                    t[axis] = value(i64::from(s));
                }
                None => remove_keeping_comments(t, axis),
            }
        }
        Some(("buttons", name)) => {
            let name = canonical_button(name);
            let t = table(doc, "buttons");
            // An alias spelling in the file would otherwise end up as a duplicate.
            for (alias, _) in BUTTON_ALIASES.iter().filter(|(_, c)| *c == name) {
                remove_keeping_comments(t, alias);
            }
            match new {
                Some(v) => t[name] = value(v),
                None => remove_keeping_comments(t, name),
            }
        }
        Some(("led", name)) => {
            let t = table(doc, "led");
            match new {
                Some(v) => t[name] = value(v),
                None => remove_keeping_comments(t, name),
            }
        }
        Some(("macros", name)) => {
            let macros = table(doc, "macros");
            macros.set_implicit(true);
            match new {
                Some(v) => {
                    let m = parse_macro_value(v)?;
                    let mut t = Table::new();
                    if m.repeat > 1 {
                        t["repeat"] = value(i64::from(m.repeat));
                    }
                    t["steps"] = value(m.steps.iter().collect::<Array>());
                    macros[name] = Item::Table(t);
                }
                None => remove_keeping_comments(macros, name),
            }
        }
        _ => return Err(err!("unknown field {key:?}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_extends() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        fs::create_dir_all(&store.profiles_dir).unwrap();
        fs::write(store.profiles_dir.join("base.toml"), "dpi = 6\n").unwrap();
        fs::write(
            store.profiles_dir.join("game.toml"),
            "# my game\n\n[buttons]\nthumb-bottom = \"key:g\"\n",
        )
        .unwrap();
        store.set_field("game", "extends", Some("base")).unwrap();
        let text = fs::read_to_string(store.profiles_dir.join("game.toml")).unwrap();
        assert!(
            text.starts_with("# my game\n\nextends = \"base\"\n\n[buttons]"),
            "{text}"
        );
        let f = store.fields("game").unwrap();
        let dpi = f.iter().find(|x| x.key == "dpi").unwrap();
        assert_eq!(
            (dpi.value.as_str(), dpi.from.as_deref()),
            ("6", Some("base"))
        );
        assert!(store.set_field("game", "extends", Some("game")).is_err());
        assert!(store.set_field("game", "extends", Some("missing")).is_err());
        store.set_field("game", "extends", Some("factory")).unwrap();
        store.set_field("game", "extends", None).unwrap();
        let text = fs::read_to_string(store.profiles_dir.join("game.toml")).unwrap();
        assert!(text.starts_with("# my game\n"), "{text}");
        assert!(!text.contains("extends"), "{text}");
    }

    #[test]
    fn polling_rate_and_sensitivity() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        fs::create_dir_all(&store.profiles_dir).unwrap();
        fs::write(
            store.profiles_dir.join("base.toml"),
            "polling-rate = 1000\n[sensitivity]\nx = 4\n",
        )
        .unwrap();
        fs::write(store.profiles_dir.join("game.toml"), "extends = \"base\"\n").unwrap();
        store.set_field("game", "sensitivity.y", Some("7")).unwrap();
        store
            .set_field("game", "polling-rate", Some("250"))
            .unwrap();
        assert!(store
            .set_field("game", "polling-rate", Some("300"))
            .is_err());
        assert!(store
            .set_field("game", "sensitivity.x", Some("11"))
            .is_err());
        let p = store.prepare("game").unwrap();
        assert_eq!(p.config.polling_rate, Some(250));
        assert_eq!(
            (p.config.sensitivity.x, p.config.sensitivity.y),
            (Some(4), Some(7))
        );
        let text = fs::read_to_string(store.profiles_dir.join("game.toml")).unwrap();
        assert!(
            text.starts_with("extends = \"base\"\npolling-rate = 250\n"),
            "{text}"
        );
        store.set_field("game", "polling-rate", None).unwrap();
        assert_eq!(
            store.prepare("game").unwrap().config.polling_rate,
            Some(1000)
        );
    }

    #[test]
    fn edit_clone_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        fs::create_dir_all(&store.profiles_dir).unwrap();
        let base = "# my base\ndpi = 3\n\n[buttons]\nthumb-bottom = \"key:g\" # for the game\n";
        fs::write(store.profiles_dir.join("default.toml"), base).unwrap();
        fs::write(
            store.profiles_dir.join("fps.toml"),
            "extends = \"default\"\n",
        )
        .unwrap();

        let f = store.fields("fps").unwrap();
        let get = |f: &[Field], k: &str| f.iter().find(|x| x.key == k).cloned().unwrap();
        assert_eq!(
            get(&f, "dpi"),
            Field {
                key: "dpi".into(),
                value: "3".into(),
                source: Source::Inherited,
                from: Some("default".into()),
            }
        );
        assert_eq!(get(&f, "buttons.left").source, Source::Default);
        assert_eq!(get(&f, "extends").value, "default");
        assert_eq!(
            get(&store.fields("default").unwrap(), "extends").source,
            Source::Default
        );
        assert_eq!(get(&f, "buttons.left").value, "left-click");

        store
            .set_field("fps", "buttons.side-right", Some("macro:hi"))
            .unwrap_err();
        store
            .set_field("fps", "macros.hi", Some("repeat=2 h 50ms i"))
            .unwrap();
        store
            .set_field("fps", "buttons.side-right", Some("macro:hi"))
            .unwrap();
        store
            .set_field("fps", "led.color", Some("#ff8000"))
            .unwrap();
        store
            .set_field("fps", "led.effect", Some("blink"))
            .unwrap_err();
        store.set_field("fps", "dpi", Some("6")).unwrap();
        let p = store.prepare("fps").unwrap();
        assert_eq!(p.config.dpi, Some(6));
        assert_eq!(p.config.macros["hi"].repeat, 2);
        assert_eq!(p.layout.slots[9][0], 0x53);
        // Removing a macro that is still used is rejected and leaves the file intact.
        store.set_field("fps", "macros.hi", None).unwrap_err();
        assert!(store.prepare("fps").is_ok());

        store.set_field("default", "dpi", None).unwrap();
        let text = fs::read_to_string(store.profiles_dir.join("default.toml")).unwrap();
        assert!(
            text.contains("# my base") && text.contains("# for the game"),
            "{text}"
        );

        store.clone_profile("fps", "fps2").unwrap();
        assert!(store.clone_profile("fps", "fps2").is_err());
        assert!(store.delete_profile("default").is_err(), "fps extends it");
        store.delete_profile("fps2").unwrap();
        assert_eq!(store.list().unwrap(), ["default", "factory", "fps"]);
    }

    #[test]
    fn factory_and_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::at(dir.path());
        assert_eq!(store.list().unwrap(), ["factory"]);
        assert_eq!(store.default_profile(), "factory");
        assert!(store.is_builtin("factory"));

        let p = store.prepare("factory").unwrap();
        assert!(p.layout.warnings.is_empty(), "{:?}", p.layout.warnings);
        assert_eq!(p.config.dpi, Some(3));
        assert_eq!(p.layout.slots[9], [0x72, 0x01, 0, 0], "illumination key");
        assert_eq!(p.layout.slots[5], [0x04, 0, 0, 0xF3], "scroll left");
        assert_eq!(
            p.layout.slots[10],
            [0x53, 0x00, 0x00, 0x0A],
            "double-click macro"
        );
        assert_eq!(
            p.layout.macros[0],
            [0x00, 0x01, 0x00, 0xF0, 0x80, 0xF0, 0x00, 0xF0, 0x80, 0xF0]
        );
        // An empty profile equals the factory layout.
        let empty = crate::config::build_layout(&Config::default()).unwrap();
        assert_eq!(empty.slots, p.layout.slots);

        assert!(store.set_field("factory", "dpi", Some("5")).is_err());
        assert!(store.delete_profile("factory").is_err());
        store.clone_profile("factory", "mine").unwrap();
        assert!(fs::read_to_string(store.profiles_dir.join("mine.toml"))
            .unwrap()
            .contains("illumination"));
        // Setting the canonical name replaces the alias spelling in the file.
        store
            .set_field("mine", "buttons.side-right", Some("copy"))
            .unwrap();
        let text = fs::read_to_string(store.profiles_dir.join("mine.toml")).unwrap();
        assert!(
            !text.contains("illumination =") && text.contains("side-right = \"copy\""),
            "{text}"
        );
        let p = store.prepare("mine").unwrap();
        assert_eq!(p.layout.slots[9][0], 0x53);
        assert_eq!(store.default_profile(), "factory");
        assert!(
            Config::parse("[buttons]\nside-right = \"none\"\nillumination = \"none\"\n").is_err()
        );
        assert_eq!(crate::config::canonical_action("menu"), "right-click");
    }
}
