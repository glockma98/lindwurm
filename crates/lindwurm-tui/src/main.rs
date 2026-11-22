use std::process::ExitCode;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use lindwurm_core::config::{
    check_macro, MacroConfig, ACTIONS, CONSUMER_CODES, LED_BRIGHTNESS, LED_EFFECT,
};
use lindwurm_core::device::Device;
use lindwurm_core::edit::{macro_value, parse_macro_value, Field, Source};
use lindwurm_core::keys::{key_code, key_names, key_warning};
use lindwurm_core::profile::Store;
use lindwurm_core::protocol::MAX_MACRO_STEPS;
use lindwurm_core::{apply_profile, Result};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};

#[derive(PartialEq)]
enum Focus {
    Profiles,
    Fields,
}

#[derive(Clone, Copy)]
enum StepKind {
    Tap,
    Press,
    Release,
}

/// Where the result of a picker goes.
#[derive(Clone)]
enum Target {
    Field(String),
    Builder(StepKind),
}

#[derive(Clone)]
enum Pick {
    Set(Option<String>),
    Keys,
    Consumers,
    Macros,
    NewMacro,
    Custom,
}

struct Picker {
    title: String,
    items: Vec<(String, Pick)>,
    filter: String,
    state: ListState,
    target: Target,
}

impl Picker {
    fn new(title: impl Into<String>, items: Vec<(String, Pick)>, target: Target) -> Picker {
        let mut state = ListState::default();
        state.select(Some(0));
        Picker {
            title: title.into(),
            items,
            filter: String::new(),
            state,
            target,
        }
    }

    fn visible(&self) -> Vec<&(String, Pick)> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|(l, _)| l.to_lowercase().contains(&f))
            .collect()
    }
}

struct Builder {
    name: Option<String>,
    /// Button field that gets the macro once it is saved.
    button: Option<String>,
    steps: Vec<String>,
    repeat: u32,
    state: ListState,
}

impl Builder {
    fn held(&self) -> Vec<String> {
        let mut held: Vec<String> = Vec::new();
        for s in &self.steps {
            if let Some(k) = s.strip_prefix('+') {
                held.push(k.to_string());
            } else if let Some(k) = s.strip_prefix('-') {
                held.retain(|h| h != k);
            }
        }
        held
    }

    fn config(&self) -> MacroConfig {
        MacroConfig {
            repeat: self.repeat,
            steps: self.steps.clone(),
        }
    }

    fn insert(&mut self, token: String) {
        let at = self
            .state
            .selected()
            .map_or(self.steps.len(), |i| i + 1)
            .min(self.steps.len());
        self.steps.insert(at, token);
        self.state.select(Some(at));
    }
}

enum Prompt {
    Edit(String),
    Clone,
    ConfirmDelete(String),
    Wait,
    Repeat,
    MacroName,
    Import,
}

enum Modal {
    Picker(Picker),
    Builder(Builder),
    Prompt(Prompt, String),
}

struct App {
    store: Store,
    profiles: Vec<String>,
    active: Option<String>,
    plist: ListState,
    fields: Vec<Field>,
    flist: ListState,
    changed: bool,
    focus: Focus,
    modals: Vec<Modal>,
    message: String,
}

fn key_items() -> Vec<(String, Pick)> {
    key_names()
        .into_iter()
        .map(|k| {
            let dropped = key_code(&k).and_then(key_warning).is_some();
            let label = if dropped {
                format!("{k}  (dropped by Linux)")
            } else {
                k.clone()
            };
            (label, Pick::Set(Some(k)))
        })
        .collect()
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(h.min(area.height))])
        .flex(Flex::Center)
        .areas(area);
    let [r] = Layout::horizontal([Constraint::Length(w.min(area.width))])
        .flex(Flex::Center)
        .areas(v);
    r
}

fn edit_hint(key: &str) -> String {
    match key {
        "extends" => "profile name, empty = none".into(),
        "dpi" => "1-7, empty = unchanged".into(),
        "polling-rate" => "1000 500 250 125 (Hz), empty = unchanged".into(),
        k if k.starts_with("sensitivity.") => "1-10 (10 = unscaled), empty = unchanged".into(),
        "led.color" => "#rrggbb, empty = unchanged".into(),
        "led.brightness" => format!("{}, empty = unchanged", LED_BRIGHTNESS.join(" ")),
        "led.effect" => format!("{}, empty = unchanged", LED_EFFECT.join(" ")),
        k if k.starts_with("macros.") => "[repeat=N] +k -k k 50ms …".into(),
        _ => "action, key:<k>, consumer:<name>, macro:<name>".into(),
    }
}

impl App {
    fn new() -> Result<App> {
        let mut app = App {
            store: Store::new()?,
            profiles: Vec::new(),
            active: None,
            plist: ListState::default(),
            fields: Vec::new(),
            flist: ListState::default(),
            changed: false,
            focus: Focus::Profiles,
            modals: Vec::new(),
            message: String::new(),
        };
        app.reload();
        if let Some(i) = app
            .active
            .as_ref()
            .and_then(|a| app.profiles.iter().position(|p| p == a))
        {
            app.plist.select(Some(i));
            app.load_fields();
        }
        Ok(app)
    }

    fn selected(&self) -> Option<String> {
        self.plist
            .selected()
            .and_then(|i| self.profiles.get(i))
            .cloned()
    }

    fn macro_names(&self) -> Vec<String> {
        self.fields
            .iter()
            .filter_map(|f| f.key.strip_prefix("macros.").map(str::to_string))
            .collect()
    }

    fn reload(&mut self) {
        match self.store.list() {
            Ok(p) => self.profiles = p,
            Err(e) => self.message = e.to_string(),
        }
        self.active = self.store.active();
        if self.profiles.is_empty() {
            self.message = format!("no profiles in {}", self.store.profiles_dir.display());
            self.plist.select(None);
        } else {
            let i = self
                .plist
                .selected()
                .unwrap_or(0)
                .min(self.profiles.len() - 1);
            self.plist.select(Some(i));
        }
        self.load_fields();
    }

    fn load_fields(&mut self) {
        let Some(name) = self.selected() else {
            self.fields.clear();
            return;
        };
        match self.store.fields(&name) {
            Ok(f) => self.fields = f,
            Err(e) => {
                self.fields.clear();
                self.message = format!("error: {e}");
            }
        }
        self.changed = self
            .store
            .prepare(&name)
            .ok()
            .and_then(|p| self.store.changed_since_applied(&p))
            == Some(true);
        if self.flist.selected().is_none_or(|i| i >= self.fields.len()) {
            self.flist.select((!self.fields.is_empty()).then_some(0));
        }
    }

    fn set(&mut self, key: &str, value: Option<&str>) -> bool {
        let Some(name) = self.selected() else {
            return false;
        };
        let r = self.store.set_field(&name, key, value);
        self.message = match &r {
            Ok(()) => format!("{key} saved"),
            Err(e) => format!("error: {e}"),
        };
        self.load_fields();
        r.is_ok()
    }

    fn open_field(&mut self, f: Field) {
        let choice = |opts: &[&str]| -> Vec<(String, Pick)> {
            let mut v = vec![("unchanged".to_string(), Pick::Set(None))];
            v.extend(
                opts.iter()
                    .map(|o| (o.to_string(), Pick::Set(Some(o.to_string())))),
            );
            v
        };
        let target = Target::Field(f.key.clone());
        let modal = match f.key.as_str() {
            "extends" => {
                let mut items = vec![("none".to_string(), Pick::Set(None))];
                let me = self.selected();
                items.extend(
                    self.profiles
                        .iter()
                        .filter(|p| Some(*p) != me.as_ref())
                        .map(|p| (p.clone(), Pick::Set(Some(p.clone())))),
                );
                Modal::Picker(Picker::new("Extends profile", items, target))
            }
            "dpi" => Modal::Picker(Picker::new(
                "DPI level",
                choice(&["1", "2", "3", "4", "5", "6", "7"]),
                target,
            )),
            "led.brightness" => Modal::Picker(Picker::new(
                "LED brightness",
                choice(LED_BRIGHTNESS),
                target,
            )),
            "led.effect" => Modal::Picker(Picker::new("LED effect", choice(LED_EFFECT), target)),
            "polling-rate" => Modal::Picker(Picker::new(
                "Polling rate (Hz)",
                choice(&["1000", "500", "250", "125"]),
                target,
            )),
            k if k.starts_with("sensitivity.") => Modal::Picker(Picker::new(
                "Sensitivity",
                choice(&["10", "9", "8", "7", "6", "5", "4", "3", "2", "1"]),
                target,
            )),
            k if k.starts_with("buttons.") => {
                let mut items: Vec<(String, Pick)> = ACTIONS
                    .iter()
                    .map(|(a, _)| (a.to_string(), Pick::Set(Some(a.to_string()))))
                    .collect();
                items.push(("key …".into(), Pick::Keys));
                items.push(("consumer key …".into(), Pick::Consumers));
                items.push(("macro …".into(), Pick::Macros));
                items.push(("custom text …".into(), Pick::Custom));
                items.push(("reset to inherited/default".into(), Pick::Set(None)));
                Modal::Picker(Picker::new(format!("{k} = {}", f.value), items, target))
            }
            k if k.starts_with("macros.") => {
                let m = parse_macro_value(&f.value).unwrap_or_default();
                let mut state = ListState::default();
                state.select(m.steps.len().checked_sub(1));
                Modal::Builder(Builder {
                    name: k.strip_prefix("macros.").map(str::to_string),
                    button: None,
                    steps: m.steps,
                    repeat: m.repeat.max(1),
                    state,
                })
            }
            _ => Modal::Prompt(Prompt::Edit(f.key.clone()), f.value.clone()),
        };
        self.modals.push(modal);
    }

    fn new_builder(&mut self, button: Option<String>) {
        self.modals.push(Modal::Builder(Builder {
            name: None,
            button,
            steps: Vec::new(),
            repeat: 1,
            state: ListState::default(),
        }));
    }

    fn save_builder(&mut self, b: Builder, name: String) {
        if self.set(&format!("macros.{name}"), Some(&macro_value(&b.config()))) {
            if let Some(button) = &b.button {
                self.set(button, Some(&format!("macro:{name}")));
            }
        } else {
            // Keep the builder open so the work is not lost.
            self.modals.push(Modal::Builder(b));
        }
    }

    fn picked(&mut self, target: Target, pick: Pick) {
        match (target, pick) {
            (Target::Field(k), Pick::Set(v)) => {
                self.set(&k, v.as_deref());
            }
            (Target::Field(k), Pick::Keys) => {
                let items = key_items()
                    .into_iter()
                    .map(|(l, p)| match p {
                        Pick::Set(Some(n)) => (l, Pick::Set(Some(format!("key:{n}")))),
                        other => (l, other),
                    })
                    .collect();
                self.modals
                    .push(Modal::Picker(Picker::new("key", items, Target::Field(k))));
            }
            (Target::Field(k), Pick::Consumers) => {
                let items = CONSUMER_CODES
                    .iter()
                    .map(|(n, c)| {
                        (
                            format!("{n}  0x{c:03X}"),
                            Pick::Set(Some(format!("consumer:{n}"))),
                        )
                    })
                    .collect();
                self.modals.push(Modal::Picker(Picker::new(
                    "consumer key",
                    items,
                    Target::Field(k),
                )));
            }
            (Target::Field(k), Pick::Macros) => {
                let mut items: Vec<(String, Pick)> = self
                    .macro_names()
                    .into_iter()
                    .map(|m| (m.clone(), Pick::Set(Some(format!("macro:{m}")))))
                    .collect();
                items.push(("new macro …".into(), Pick::NewMacro));
                self.modals
                    .push(Modal::Picker(Picker::new("macro", items, Target::Field(k))));
            }
            (Target::Field(k), Pick::NewMacro) => self.new_builder(Some(k)),
            (Target::Field(k), Pick::Custom) => {
                let v = self
                    .fields
                    .iter()
                    .find(|f| f.key == k)
                    .map(|f| f.value.clone())
                    .unwrap_or_default();
                self.modals.push(Modal::Prompt(Prompt::Edit(k), v));
            }
            (Target::Builder(kind), Pick::Set(Some(key))) => {
                if let Some(Modal::Builder(b)) = self.modals.last_mut() {
                    b.insert(match kind {
                        StepKind::Tap => key,
                        StepKind::Press => format!("+{key}"),
                        StepKind::Release => format!("-{key}"),
                    });
                }
            }
            _ => {}
        }
    }

    fn finish_prompt(&mut self, p: Prompt, input: String) {
        let name = self.selected().unwrap_or_default();
        let input = input.trim().to_string();
        match p {
            Prompt::Edit(key) => {
                self.set(&key, Some(&input));
            }
            Prompt::Clone => {
                self.message = match self.store.clone_profile(&name, &input) {
                    Ok(()) => format!("cloned {name} to {input}"),
                    Err(e) => format!("error: {e}"),
                };
                self.reload();
                if let Some(i) = self.profiles.iter().position(|p| *p == input) {
                    self.plist.select(Some(i));
                    self.load_fields();
                }
            }
            Prompt::ConfirmDelete(n) => {
                if input == "y" {
                    self.message = match self.store.delete_profile(&n) {
                        Ok(()) => format!("deleted {n}"),
                        Err(e) => format!("error: {e}"),
                    };
                    self.reload();
                }
            }
            Prompt::Wait => {
                let ok = input
                    .parse::<u32>()
                    .ok()
                    .filter(|ms| *ms > 0 && ms % 50 == 0 && *ms <= 6350);
                match (ok, self.modals.last_mut()) {
                    (Some(ms), Some(Modal::Builder(b))) => b.insert(format!("{ms}ms")),
                    _ => self.message = "error: pause must be 50-6350 ms in steps of 50".into(),
                }
            }
            Prompt::Repeat => {
                let ok = input
                    .parse::<u32>()
                    .ok()
                    .filter(|r| (1..=32767).contains(r));
                match (ok, self.modals.last_mut()) {
                    (Some(r), Some(Modal::Builder(b))) => b.repeat = r,
                    _ => self.message = "error: repeat must be 1-32767".into(),
                }
            }
            Prompt::Import => {
                // "path [prefix]"; a last word counts as prefix unless it looks like a path.
                let (path, prefix) = match input.rsplit_once(' ') {
                    Some((p, x)) if !x.contains('/') && !x.ends_with(".ini") => (p.trim(), x),
                    _ => (input.as_str(), "ini"),
                };
                let path = match path.strip_prefix("~/") {
                    Some(rest) => std::env::var("HOME")
                        .map(|h| format!("{h}/{rest}"))
                        .unwrap_or(path.to_string()),
                    None => path.to_string(),
                };
                self.message = match self
                    .store
                    .import_ini_file(std::path::Path::new(&path), prefix)
                {
                    Ok(r) => Store::import_summary(&r),
                    Err(e) => format!("error: {e}"),
                };
                self.reload();
            }
            Prompt::MacroName => {
                let valid = !input.is_empty()
                    && input
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                if !valid {
                    self.message = "error: macro names use letters, digits, - and _".into();
                    return;
                }
                if let Some(Modal::Builder(b)) = self.modals.pop() {
                    self.save_builder(b, input);
                }
            }
        }
    }

    fn builder_key(&mut self, code: KeyCode) {
        let Some(Modal::Builder(b)) = self.modals.last_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                self.modals.pop();
            }
            KeyCode::Down | KeyCode::Char('j') => b.state.select_next(),
            KeyCode::Up | KeyCode::Char('k') => b.state.select_previous(),
            KeyCode::Char('x') | KeyCode::Delete | KeyCode::Backspace => {
                if let Some(i) = b.state.selected().filter(|i| *i < b.steps.len()) {
                    b.steps.remove(i);
                    b.state
                        .select(i.checked_sub(1).or((!b.steps.is_empty()).then_some(0)));
                }
            }
            KeyCode::Char('t') => self.modals.push(Modal::Picker(Picker::new(
                "tap key",
                key_items(),
                Target::Builder(StepKind::Tap),
            ))),
            KeyCode::Char('p') => self.modals.push(Modal::Picker(Picker::new(
                "press and hold key",
                key_items(),
                Target::Builder(StepKind::Press),
            ))),
            KeyCode::Char('r') => {
                let held: Vec<(String, Pick)> = b
                    .held()
                    .into_iter()
                    .map(|k| (k.clone(), Pick::Set(Some(k))))
                    .collect();
                if held.is_empty() {
                    self.message = "no key is held at this point".into();
                } else {
                    self.modals.push(Modal::Picker(Picker::new(
                        "release key",
                        held,
                        Target::Builder(StepKind::Release),
                    )));
                }
            }
            KeyCode::Char('w') => self.modals.push(Modal::Prompt(Prompt::Wait, "100".into())),
            KeyCode::Char('n') => {
                let r = b.repeat.to_string();
                self.modals.push(Modal::Prompt(Prompt::Repeat, r));
            }
            KeyCode::Enter | KeyCode::Char('s') => {
                if let Err(e) = check_macro(&b.config()) {
                    self.message = format!("error: {e}");
                    return;
                }
                match b.name.clone() {
                    Some(name) => {
                        if let Some(Modal::Builder(b)) = self.modals.pop() {
                            self.save_builder(b, name);
                        }
                    }
                    None => self
                        .modals
                        .push(Modal::Prompt(Prompt::MacroName, String::new())),
                }
            }
            _ => {}
        }
    }

    fn picker_key(&mut self, code: KeyCode) {
        let Some(Modal::Picker(pk)) = self.modals.last_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => {
                self.modals.pop();
            }
            KeyCode::Down => pk.state.select_next(),
            KeyCode::Up => pk.state.select_previous(),
            KeyCode::Backspace => {
                pk.filter.pop();
                pk.state.select(Some(0));
            }
            KeyCode::Char(c) => {
                pk.filter.push(c);
                pk.state.select(Some(0));
            }
            KeyCode::Enter => {
                let chosen = pk
                    .state
                    .selected()
                    .and_then(|i| pk.visible().get(i).map(|(_, p)| p.clone()));
                let target = pk.target.clone();
                let Some(pick) = chosen else { return };
                self.modals.pop();
                // A final value for a field closes the whole picker chain.
                if matches!(pick, Pick::Set(_)) && matches!(target, Target::Field(_)) {
                    while matches!(self.modals.last(), Some(Modal::Picker(_))) {
                        self.modals.pop();
                    }
                }
                self.picked(target, pick);
            }
            _ => {}
        }
    }

    fn prompt_key(&mut self, code: KeyCode) {
        let Some(Modal::Prompt(p, input)) = self.modals.last_mut() else {
            return;
        };
        let confirm = matches!(p, Prompt::ConfirmDelete(_));
        match code {
            KeyCode::Esc => {
                self.modals.pop();
                return;
            }
            KeyCode::Backspace => {
                input.pop();
                return;
            }
            KeyCode::Char(c) => {
                input.push(c);
                if !confirm {
                    return;
                }
            }
            KeyCode::Enter => {}
            _ => return,
        }
        if let Some(Modal::Prompt(p, input)) = self.modals.pop() {
            self.finish_prompt(p, input);
        }
    }

    fn on_key(&mut self, code: KeyCode, terminal: &mut DefaultTerminal) -> bool {
        match self.modals.last() {
            Some(Modal::Prompt(..)) => {
                return {
                    self.prompt_key(code);
                    true
                }
            }
            Some(Modal::Picker(_)) => {
                return {
                    self.picker_key(code);
                    true
                }
            }
            Some(Modal::Builder(_)) => {
                return {
                    self.builder_key(code);
                    true
                }
            }
            None => {}
        }
        let current = self
            .flist
            .selected()
            .and_then(|i| self.fields.get(i))
            .cloned();
        match (code, &self.focus) {
            (KeyCode::Char('q') | KeyCode::Esc, _) => return false,
            (KeyCode::Tab | KeyCode::BackTab | KeyCode::Right, Focus::Profiles) => {
                self.focus = Focus::Fields
            }
            (KeyCode::Tab | KeyCode::BackTab | KeyCode::Left, Focus::Fields) => {
                self.focus = Focus::Profiles
            }
            (KeyCode::Down | KeyCode::Char('j'), Focus::Profiles) => {
                self.plist.select_next();
                self.load_fields();
            }
            (KeyCode::Up | KeyCode::Char('k'), Focus::Profiles) => {
                self.plist.select_previous();
                self.load_fields();
            }
            (KeyCode::Enter, Focus::Profiles) => self.apply_selected(terminal),
            (KeyCode::Char('i'), Focus::Profiles) => self
                .modals
                .push(Modal::Prompt(Prompt::Import, String::new())),
            (KeyCode::Char('c'), Focus::Profiles) => self
                .modals
                .push(Modal::Prompt(Prompt::Clone, String::new())),
            (KeyCode::Char('D'), Focus::Profiles) => {
                if let Some(n) = self.selected() {
                    self.modals
                        .push(Modal::Prompt(Prompt::ConfirmDelete(n), String::new()));
                }
            }
            (KeyCode::Char('r'), _) => {
                self.reload();
                self.message = "reloaded".into();
            }
            (KeyCode::Char(c @ '1'..='7'), Focus::Profiles) => {
                let level = c as u8 - b'0';
                self.message = match Device::open().and_then(|d| d.set_dpi(level)) {
                    Ok(()) => format!("dpi level {level} set"),
                    Err(e) => format!("error: {e}"),
                };
            }
            (KeyCode::Down | KeyCode::Char('j'), Focus::Fields) => self.flist.select_next(),
            (KeyCode::Up | KeyCode::Char('k'), Focus::Fields) => self.flist.select_previous(),
            (KeyCode::Enter, Focus::Fields) => {
                if let Some(f) = current {
                    self.open_field(f);
                }
            }
            (KeyCode::Char('e'), Focus::Fields) => {
                if let Some(f) = current {
                    self.modals
                        .push(Modal::Prompt(Prompt::Edit(f.key), f.value));
                }
            }
            (KeyCode::Char('x') | KeyCode::Delete, Focus::Fields) => {
                if let Some(f) = current {
                    self.set(&f.key, None);
                }
            }
            (KeyCode::Char('a'), Focus::Fields) => self.new_builder(None),
            _ => {}
        }
        true
    }

    fn apply_selected(&mut self, terminal: &mut DefaultTerminal) {
        let Some(name) = self.selected() else { return };
        self.message = format!("writing {name} …");
        // Writing takes about two seconds; show the message before blocking.
        let _ = terminal.draw(|f| self.draw(f));
        self.message = match apply_profile(&self.store, &name) {
            Ok(p) if p.layout.warnings.is_empty() => format!("profile {name} written"),
            Ok(p) => format!(
                "profile {name} written, warning: {}",
                p.layout.warnings.join("; ")
            ),
            Err(e) => format!("error: {e}"),
        };
        self.active = self.store.active();
        self.load_fields();
    }

    fn status_lines(&self) -> (String, String) {
        match self.modals.last() {
            Some(Modal::Prompt(p, input)) => {
                let (label, hint) = match p {
                    Prompt::Edit(k) => (format!("{k} = "), edit_hint(k)),
                    Prompt::Clone => ("clone as: ".into(), String::new()),
                    Prompt::ConfirmDelete(n) => (format!("delete {n}? (y/n) "), String::new()),
                    Prompt::Wait => ("pause in ms: ".into(), "50-6350, steps of 50".into()),
                    Prompt::Repeat => ("repeat count: ".into(), "1-32767".into()),
                    Prompt::MacroName => ("macro name: ".into(), "letters, digits, - and _".into()),
                    Prompt::Import => (
                        "import Mouse.ini or .conf: ".into(),
                        "path, optionally followed by a name prefix (default ini)".into(),
                    ),
                };
                (format!("{label}{input}▏"), format!("{hint}  · enter confirm · esc cancel"))
            }
            Some(Modal::Picker(_)) => {
                (self.message.clone(), "type to filter · ↑↓ select · enter choose · esc back".into())
            }
            Some(Modal::Builder(_)) => (
                self.message.clone(),
                "t tap · p hold · r release · w pause · x remove · n repeat · enter save · esc cancel".into(),
            ),
            None => (
                self.message.clone(),
                match self.focus {
                    Focus::Profiles => {
                        "enter apply · c clone · D delete · i import ini · 1-7 dpi · tab fields · r reload · q quit".into()
                    }
                    Focus::Fields => {
                        "enter choose · e edit text · x reset · a new macro · tab profiles · q quit".into()
                    }
                },
            ),
        }
    }

    fn draw(&mut self, f: &mut Frame) {
        let [main, bottom] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(4)]).areas(f.area());
        let [left, right] =
            Layout::horizontal([Constraint::Length(24), Constraint::Min(30)]).areas(main);
        let dim = Style::new().fg(Color::DarkGray);
        let hl = Style::new().add_modifier(Modifier::REVERSED);
        let accent = Style::new().fg(Color::Cyan);
        // The profile stays marked while its fields are edited; the field
        // cursor only shows while the fields have the focus.
        let p_hl = if self.focus == Focus::Profiles {
            hl
        } else {
            Style::new().add_modifier(Modifier::BOLD)
        };
        let f_hl = if self.focus == Focus::Fields {
            hl
        } else {
            Style::new()
        };
        let no_modal = self.modals.is_empty();

        let items: Vec<ListItem> = self
            .profiles
            .iter()
            .map(|p| {
                ListItem::new(format!(
                    "{}{p}",
                    if Some(p) == self.active.as_ref() {
                        "* "
                    } else {
                        "  "
                    }
                ))
            })
            .collect();
        let pborder = if no_modal && self.focus == Focus::Profiles {
            accent
        } else {
            Style::new()
        };
        let list = List::new(items)
            .block(Block::bordered().title(" profiles ").border_style(pborder))
            .highlight_style(p_hl);
        f.render_stateful_widget(list, left, &mut self.plist);

        let rows: Vec<ListItem> = self
            .fields
            .iter()
            .map(|fl| {
                let (value, style, note) = match fl.source {
                    Source::Own => (fl.value.as_str(), Style::new(), String::new()),
                    Source::Inherited => (
                        fl.value.as_str(),
                        Style::new(),
                        format!("  ↑ {}", fl.from.as_deref().unwrap_or_default()),
                    ),
                    Source::Default if fl.key == "extends" => ("none", dim, String::new()),
                    Source::Default if fl.value.is_empty() => ("unchanged", dim, String::new()),
                    Source::Default => (fl.value.as_str(), dim, String::new()),
                };
                ListItem::new(Line::from(vec![
                    Span::raw(format!("{:<20} ", fl.key)),
                    Span::styled(value.to_string(), style),
                    Span::styled(note, dim),
                ]))
            })
            .collect();
        let mut title = format!(" {} ", self.selected().unwrap_or_default());
        if self.changed {
            title += "· changed since written ";
        }
        let fborder = if no_modal && self.focus == Focus::Fields {
            accent
        } else {
            Style::new()
        };
        let fields = List::new(rows)
            .block(
                Block::bordered()
                    .title(title)
                    .title_bottom(" ↑ inherited from · grey = default ")
                    .border_style(fborder),
            )
            .highlight_style(f_hl);
        f.render_stateful_widget(fields, right, &mut self.flist);

        let (first, help) = self.status_lines();
        f.render_widget(
            Paragraph::new(vec![Line::from(first), Line::styled(help, dim)])
                .block(Block::bordered()),
            bottom,
        );

        // Only the topmost picker or builder is drawn; prompts use the status line.
        match self
            .modals
            .iter_mut()
            .rev()
            .find(|m| !matches!(m, Modal::Prompt(..)))
        {
            Some(Modal::Picker(pk)) => {
                let area = centered(main, 50, 20);
                f.render_widget(Clear, area);
                let items: Vec<ListItem> = pk
                    .visible()
                    .iter()
                    .map(|(l, _)| ListItem::new(l.clone()))
                    .collect();
                let title = if pk.filter.is_empty() {
                    format!(" {} ", pk.title)
                } else {
                    format!(" {} · filter: {} ", pk.title, pk.filter)
                };
                let list = List::new(items)
                    .block(Block::bordered().title(title).border_style(accent))
                    .highlight_style(hl);
                f.render_stateful_widget(list, area, &mut pk.state);
            }
            Some(Modal::Builder(b)) => {
                let area = centered(main, 56, 22);
                f.render_widget(Clear, area);
                let status = match check_macro(&b.config()) {
                    Ok(n) => format!("{n}/{} steps", MAX_MACRO_STEPS - 1),
                    Err(e) => e.to_string(),
                };
                let name = b.name.clone().unwrap_or_else(|| "new macro".into());
                let items: Vec<ListItem> = b
                    .steps
                    .iter()
                    .map(|s| {
                        ListItem::new(if let Some(k) = s.strip_prefix('+') {
                            format!("hold     {k}")
                        } else if let Some(k) = s.strip_prefix('-') {
                            format!("release  {k}")
                        } else if s.ends_with("ms") {
                            format!("  pause  {s}")
                        } else {
                            format!("tap      {s}")
                        })
                    })
                    .collect();
                let block = Block::bordered()
                    .title(format!(" {name} · repeat {} ", b.repeat))
                    .title_bottom(format!(" {status} "))
                    .border_style(accent);
                f.render_stateful_widget(
                    List::new(items).block(block).highlight_style(hl),
                    area,
                    &mut b.state,
                );
            }
            _ => {}
        }
    }

    fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Press && !self.on_key(key.code, terminal) {
                return Ok(());
            }
        }
    }
}

fn main() -> ExitCode {
    let mut app = match App::new() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("lindwurm-tui: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lindwurm-tui: {e}");
            ExitCode::FAILURE
        }
    }
}
