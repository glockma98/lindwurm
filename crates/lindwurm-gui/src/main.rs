// No console window next to the GUI on Windows (release builds only, so
// debug builds keep their log output).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver};
use std::thread;

use eframe::egui;
use lindwurm_core::config::{parse_color, ACTIONS, CONSUMER_CODES, LED_BRIGHTNESS, LED_EFFECT};
use lindwurm_core::device::Device;
use lindwurm_core::edit::{Field, Source};
use lindwurm_core::profile::Store;
use lindwurm_core::{apply_profile, status_text};

struct App {
    store: Store,
    profiles: Vec<String>,
    active: Option<String>,
    selected: Option<String>,
    fields: Vec<Field>,
    changed: bool,
    // Text being edited per field key; saved on Enter or when focus leaves.
    edits: HashMap<String, String>,
    clone_name: String,
    new_macro: (String, String),
    confirm_delete: bool,
    import_path: String,
    import_prefix: String,
    message: String,
    // Writing takes about two seconds, so it runs off the UI thread.
    pending: Option<Receiver<String>>,
}

impl App {
    fn new(store: Store) -> App {
        let mut app = App {
            store,
            profiles: Vec::new(),
            active: None,
            selected: None,
            fields: Vec::new(),
            changed: false,
            edits: HashMap::new(),
            clone_name: String::new(),
            new_macro: Default::default(),
            confirm_delete: false,
            import_path: String::new(),
            import_prefix: "ini".into(),
            message: String::new(),
            pending: None,
        };
        app.reload();
        app.selected = app.active.clone().or_else(|| app.profiles.first().cloned());
        app.load_fields();
        app
    }

    fn reload(&mut self) {
        match self.store.list() {
            Ok(p) => self.profiles = p,
            Err(e) => self.message = e.to_string(),
        }
        self.active = self.store.active();
        if self
            .selected
            .as_ref()
            .is_some_and(|s| !self.profiles.contains(s))
        {
            self.selected = self.profiles.first().cloned();
        }
        if self.profiles.is_empty() {
            self.message = format!("no profiles in {}", self.store.profiles_dir.display());
        }
        self.load_fields();
    }

    fn load_fields(&mut self) {
        self.edits.clear();
        self.confirm_delete = false;
        let Some(name) = self.selected.clone() else {
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
    }

    fn set(&mut self, key: &str, value: Option<&str>) {
        let Some(name) = self.selected.clone() else {
            return;
        };
        self.message = match self.store.set_field(&name, key, value) {
            Ok(()) => format!("{key} saved"),
            Err(e) => format!("error: {e}"),
        };
        self.load_fields();
    }

    fn spawn(
        &mut self,
        ctx: &egui::Context,
        label: String,
        job: impl FnOnce() -> String + Send + 'static,
    ) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        self.message = label;
        self.pending = Some(rx);
        thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
    }

    fn profiles_panel(&mut self, ui: &mut egui::Ui, busy: bool) {
        ui.heading("Profiles");
        let mut pick = None;
        for p in &self.profiles {
            let label = if Some(p) == self.active.as_ref() {
                format!("{p}  ★")
            } else {
                p.clone()
            };
            if ui
                .selectable_label(Some(p) == self.selected.as_ref(), label)
                .clicked()
            {
                pick = Some(p.clone());
            }
        }
        if pick.is_some() && pick != self.selected {
            self.selected = pick;
            self.load_fields();
        }
        ui.weak("★ = written to the mouse last");
        ui.separator();
        let Some(name) = self.selected.clone() else {
            return;
        };
        if ui
            .add_enabled(!busy, egui::Button::new(format!("Apply {name}")))
            .clicked()
        {
            let store = self.store.clone();
            let n = name.clone();
            self.spawn(
                ui.ctx(),
                format!("writing {name} …"),
                move || match apply_profile(&store, &n) {
                    Ok(p) if p.layout.warnings.is_empty() => format!("profile {n} written"),
                    Ok(p) => format!(
                        "profile {n} written, warning: {}",
                        p.layout.warnings.join("; ")
                    ),
                    Err(e) => format!("error: {e}"),
                },
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.clone_name)
                    .hint_text("new name")
                    .desired_width(100.0),
            );
            if ui.button("Clone").clicked() {
                let to = self.clone_name.trim().to_string();
                self.message = match self.store.clone_profile(&name, &to) {
                    Ok(()) => {
                        self.clone_name.clear();
                        self.selected = Some(to.clone());
                        format!("cloned {name} to {to}")
                    }
                    Err(e) => format!("error: {e}"),
                };
                self.reload();
            }
        });
        ui.add_space(8.0);
        if self.confirm_delete {
            ui.horizontal(|ui| {
                ui.label(format!("Delete {name}?"));
                if ui.button("Yes").clicked() {
                    self.message = match self.store.delete_profile(&name) {
                        Ok(()) => format!("deleted {name}"),
                        Err(e) => format!("error: {e}"),
                    };
                    self.selected = None;
                    self.reload();
                }
                if ui.button("No").clicked() {
                    self.confirm_delete = false;
                }
            });
        } else if ui.button("Delete…").clicked() {
            self.confirm_delete = true;
        }
    }

    fn import_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Import Mouse.ini").show(ui, |ui| {
            ui.weak("profiles of the vendor's Windows software");
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.import_path)
                        .hint_text("Mouse.ini or .conf")
                        .desired_width(110.0),
                );
                if ui.button("Browse…").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Mouse.ini or exported profile", &["ini", "INI", "conf"])
                        .pick_file()
                    {
                        self.import_path = p.display().to_string();
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("name prefix");
                ui.add(egui::TextEdit::singleline(&mut self.import_prefix).desired_width(60.0));
            });
            let ready =
                !self.import_path.trim().is_empty() && !self.import_prefix.trim().is_empty();
            if ui.add_enabled(ready, egui::Button::new("Import")).clicked() {
                let path = std::path::PathBuf::from(self.import_path.trim());
                self.message = match self.store.import_ini_file(&path, self.import_prefix.trim()) {
                    Ok(r) => {
                        self.selected = r.first().map(|(n, _)| n.clone());
                        Store::import_summary(&r)
                    }
                    Err(e) => format!("error: {e}"),
                };
                self.reload();
            }
        });
    }

    fn source_note(ui: &mut egui::Ui, f: &Field) {
        match f.source {
            Source::Own => {}
            Source::Inherited => {
                ui.weak(format!("from {}", f.from.as_deref().unwrap_or("?")));
            }
            Source::Default => {
                ui.weak("default");
            }
        }
    }

    fn reset_button(&mut self, ui: &mut egui::Ui, f: &Field) {
        if f.source == Source::Own
            && ui
                .small_button("reset")
                .on_hover_text("use the inherited or default value")
                .clicked()
        {
            self.set(&f.key, None);
        }
    }

    fn choice(&mut self, ui: &mut egui::Ui, f: &Field, options: &[&str]) {
        self.choice_with(ui, f, options, "unchanged");
    }

    /// `empty` names the choice that removes the field.
    fn choice_with(&mut self, ui: &mut egui::Ui, f: &Field, options: &[&str], empty: &str) {
        let shown = if f.value.is_empty() {
            empty
        } else {
            f.value.as_str()
        };
        let mut pick: Option<Option<String>> = None;
        egui::ComboBox::from_id_salt(&f.key)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(f.value.is_empty(), empty).clicked() {
                    pick = Some(None);
                }
                for o in options {
                    if ui.selectable_label(f.value == *o, *o).clicked() {
                        pick = Some(Some(o.to_string()));
                    }
                }
            });
        if let Some(v) = pick {
            self.set(&f.key, v.as_deref());
        }
    }

    fn text_field(&mut self, ui: &mut egui::Ui, f: &Field, width: f32) {
        let buf = self
            .edits
            .entry(f.key.clone())
            .or_insert_with(|| f.value.clone());
        let r = ui.add(egui::TextEdit::singleline(buf).desired_width(width));
        let commit = r.lost_focus() && *buf != f.value;
        if commit {
            let v = buf.clone();
            self.set(&f.key, Some(&v));
        }
    }

    fn action_menu(&mut self, ui: &mut egui::Ui, f: &Field) {
        let macros: Vec<String> = self
            .fields
            .iter()
            .filter_map(|x| x.key.strip_prefix("macros.").map(str::to_string))
            .collect();
        let mut pick = None;
        ui.menu_button("▾", |ui| {
            for (a, _) in ACTIONS {
                if ui.button(*a).clicked() {
                    pick = Some(a.to_string());
                }
            }
            ui.menu_button("consumer", |ui| {
                for (c, _) in CONSUMER_CODES {
                    if ui.button(*c).clicked() {
                        pick = Some(format!("consumer:{c}"));
                    }
                }
            });
            if !macros.is_empty() {
                ui.menu_button("macro", |ui| {
                    for m in &macros {
                        if ui.button(m).clicked() {
                            pick = Some(format!("macro:{m}"));
                        }
                    }
                });
            }
            ui.weak("keys: type key:<name>");
        });
        if let Some(a) = pick {
            self.set(&f.key, Some(&a));
        }
    }

    fn fields_panel(&mut self, ui: &mut egui::Ui) {
        let Some(name) = self.selected.clone() else {
            ui.label("no profile selected");
            return;
        };
        ui.horizontal(|ui| {
            ui.heading(&name);
            if self.changed {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "changed since written – apply again",
                );
            }
        });
        let fields = self.fields.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("fields")
                .num_columns(4)
                .striped(true)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for f in &fields {
                        ui.label(&f.key);
                        match f.key.as_str() {
                            "extends" => {
                                let others: Vec<String> = self
                                    .profiles
                                    .iter()
                                    .filter(|p| Some(*p) != self.selected.as_ref())
                                    .cloned()
                                    .collect();
                                let others: Vec<&str> = others.iter().map(String::as_str).collect();
                                self.choice_with(ui, f, &others, "none");
                            }
                            "dpi" => self.choice(ui, f, &["1", "2", "3", "4", "5", "6", "7"]),
                            "polling-rate" => self.choice(ui, f, &["1000", "500", "250", "125"]),
                            k if k.starts_with("sensitivity.") => self.choice(
                                ui,
                                f,
                                &["10", "9", "8", "7", "6", "5", "4", "3", "2", "1"],
                            ),
                            "led.brightness" => self.choice(ui, f, LED_BRIGHTNESS),
                            "led.effect" => self.choice(ui, f, LED_EFFECT),
                            "led.color" => {
                                ui.horizontal(|ui| match parse_color(&f.value) {
                                    Ok(mut rgb) => {
                                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                                            let v = format!(
                                                "#{:02x}{:02x}{:02x}",
                                                rgb[0], rgb[1], rgb[2]
                                            );
                                            self.set(&f.key, Some(&v));
                                        }
                                        ui.monospace(&f.value);
                                    }
                                    Err(_) => {
                                        ui.weak("unchanged");
                                        if ui.small_button("set").clicked() {
                                            self.set(&f.key, Some("#00ff00"));
                                        }
                                    }
                                });
                            }
                            k if k.starts_with("buttons.") => {
                                ui.horizontal(|ui| {
                                    self.text_field(ui, f, 200.0);
                                    self.action_menu(ui, f);
                                });
                            }
                            _ => {
                                self.text_field(ui, f, 280.0);
                            }
                        }
                        Self::source_note(ui, f);
                        if f.key.starts_with("macros.") {
                            if f.source == Source::Own && ui.small_button("delete").clicked() {
                                self.set(&f.key, None);
                            }
                        } else {
                            self.reset_button(ui, f);
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("new macro");
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_macro.0)
                        .hint_text("name")
                        .desired_width(80.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_macro.1)
                        .hint_text("[repeat=N] +k -k k 50ms …")
                        .desired_width(220.0),
                );
                if ui.button("Add").clicked() {
                    let (n, s) = std::mem::take(&mut self.new_macro);
                    self.set(&format!("macros.{}", n.trim()), Some(&s));
                }
            });
            ui.add_space(8.0);
            egui::CollapsingHeader::new("Last written to the mouse").show(ui, |ui| {
                let text = status_text(&self.store)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "nothing written by lindwurm yet".into());
                ui.monospace(text);
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.pending {
            if let Ok(msg) = rx.try_recv() {
                self.message = msg;
                self.pending = None;
                self.active = self.store.active();
                self.load_fields();
            }
        }
        let busy = self.pending.is_some();
        egui::Panel::bottom("status").show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label("DPI now");
                for level in 1..=7u8 {
                    if ui
                        .add_enabled(!busy, egui::Button::new(level.to_string()))
                        .clicked()
                    {
                        self.message = match Device::open().and_then(|d| d.set_dpi(level)) {
                            Ok(()) => format!("dpi level {level} set"),
                            Err(e) => format!("error: {e}"),
                        };
                    }
                }
                ui.separator();
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                if busy {
                    ui.spinner();
                }
                ui.label(&self.message);
            });
        });
        egui::Panel::left("profiles")
            .default_size(190.0)
            .show(root, |ui| {
                self.profiles_panel(ui, busy);
                ui.separator();
                self.import_panel(ui);
            });
        egui::CentralPanel::default().show(root, |ui| self.fields_panel(ui));
    }
}

fn main() -> eframe::Result {
    let store = match Store::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lindwurm-gui: {e}");
            std::process::exit(1);
        }
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([820.0, 620.0]),
        ..Default::default()
    };
    eframe::run_native(
        "lindwurm",
        options,
        Box::new(|_cc| Ok(Box::new(App::new(store)))),
    )
}
