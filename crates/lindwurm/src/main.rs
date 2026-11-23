use std::process::ExitCode;

use lindwurm_core::config::{action_help, button_slots, BUTTONS, BUTTON_ALIASES};
use lindwurm_core::device::Device;
use lindwurm_core::edit::Source;
use lindwurm_core::profile::Store;
use lindwurm_core::protocol::describe_layout;
use lindwurm_core::{apply_profile, status_text, Error, Result};

const USAGE: &str = "usage: lindwurm <command> [args]

  apply [--quick] [profile]
                   write a profile (buttons, macros, DPI, LED, ...) to the mouse;
                   --quick skips buttons and macros if they match the last
                   write from this computer (not if the mouse was written
                   elsewhere since)
  show [profile]   show all settings, buttons and macros of a profile
  set <profile> <field> [value]
                   change a field (as listed by show); without value the
                   inherited or default value applies again
  clone <from> <to>
                   copy a profile
  delete <profile> remove a profile
  udev-rule        print the udev rule that gives access on Linux; install with
                   lindwurm udev-rule | sudo tee /etc/udev/rules.d/70-lindwurm.rules
  import <file> [prefix]
                   convert the vendor's Mouse.ini or .conf export into profiles
                   <prefix>-1 .. <prefix>-5 (prefix defaults to \"ini\")
  check [profile]  validate a profile and print the raw slots, write nothing
  profiles         list profiles, * marks the one written last
  status           show the profile written last and its slots
  dpi <1-7>        set the DPI level
  polling-rate <hz>
                   set the polling rate (1000, 500, 250 or 125); the mouse
                   reconnects for a moment
  sensitivity <x> [y]
                   set the speed of each axis, 1-10 (10 = unscaled)
  rescue           if the pointer stopped moving: normal mode and full speed on
                   both axes; buttons, macros, DPI and LED stay as they are
  buttons          list button names for the [buttons] table
  actions          list built-in actions
  version          print the version

A profile is a name in the profiles directory (see `lindwurm profiles`) or a
path to a .toml file; it defaults to \"default\", or to the built-in \"factory\" profile when
there is no \"default\". A profile may set
extends = \"<profile>\" to only override parts of another one.
";

fn run(args: &[String]) -> Result<()> {
    let store = Store::new()?;
    let fallback = store.default_profile();
    let profile = args.get(1).map(String::as_str).unwrap_or(&fallback);
    match args.first().map(String::as_str) {
        None | Some("help" | "-h" | "--help") => print!("{USAGE}"),
        Some("version" | "-V" | "--version") => println!("lindwurm {}", env!("CARGO_PKG_VERSION")),
        Some("check") => {
            let p = store.prepare(profile)?;
            for w in &p.layout.warnings {
                eprintln!("warning: {w}");
            }
            if let Some(d) = p.config.dpi {
                println!("dpi      {d}");
            }
            print!("{}", describe_layout(&p.layout, &button_slots()));
        }
        Some("show") => {
            let p = store.prepare(profile)?;
            println!("{:<20} {}", "profile", p.name);
            for f in store.fields(profile)? {
                let note = match f.source {
                    Source::Own => String::new(),
                    Source::Inherited => format!("  (from {})", f.from.as_deref().unwrap_or("?")),
                    Source::Default if f.key == "extends" => "none".into(),
                    Source::Default if f.value.is_empty() => "unchanged".into(),
                    Source::Default => "  (default)".into(),
                };
                println!("{:<20} {}{note}", f.key, f.value);
            }
            if store.changed_since_applied(&p) == Some(true) {
                println!("note: the file changed since it was written, apply it again");
            }
        }
        Some("set") => {
            let field = args
                .get(2)
                .ok_or_else(|| Error("set needs <profile> <field> [value]".into()))?;
            let value = (args.len() > 3).then(|| args[3..].join(" "));
            store.set_field(profile, field, value.as_deref())?;
        }
        Some("clone") => {
            let to = args
                .get(2)
                .ok_or_else(|| Error("clone needs <from> <to>".into()))?;
            store.clone_profile(profile, to)?;
        }
        Some("import") => {
            let path = args.get(1).ok_or_else(|| {
                Error("import needs the path to Mouse.ini or a .conf export".into())
            })?;
            let prefix = args.get(2).map(String::as_str).unwrap_or("ini");
            for (name, i) in store.import_ini_file(std::path::Path::new(path), prefix)? {
                let note = if i.same_as_factory {
                    "  (same as factory)"
                } else {
                    ""
                };
                println!("{} -> {name}{note}", i.section);
                for w in &i.warnings {
                    eprintln!("warning: {w}");
                }
            }
        }
        Some("delete") => {
            if args.len() < 2 {
                return Err(Error("delete needs a profile name".into()));
            }
            store.delete_profile(profile)?;
        }
        Some("apply") => {
            let quick = args.iter().any(|a| a == "--quick");
            let name = args[1..]
                .iter()
                .find(|a| *a != "--quick")
                .map(String::as_str)
                .unwrap_or(&fallback);
            let (p, skipped) = apply_profile(&store, name, quick)?;
            for w in &p.layout.warnings {
                eprintln!("warning: {w}");
            }
            match skipped {
                true => println!("profile {} written (buttons and macros unchanged)", p.name),
                false => println!("profile {} written", p.name),
            }
        }
        Some("profiles") => {
            let active = store.active();
            for n in store.list()? {
                let mark = if Some(&n) == active.as_ref() {
                    "*"
                } else {
                    " "
                };
                println!("{mark} {n}");
            }
            println!("(profiles folder: {})", store.profiles_dir.display());
        }
        Some("status") => match status_text(&store)? {
            Some(s) => println!("{s}"),
            None => println!("nothing written by lindwurm yet"),
        },
        Some("dpi") => {
            let level = args
                .get(1)
                .and_then(|a| a.parse().ok())
                .ok_or_else(|| Error("dpi needs a level from 1 to 7".into()))?;
            Device::open()?.set_dpi(level)?;
        }
        Some("polling-rate") => {
            let hz = args
                .get(1)
                .and_then(|a| a.parse().ok())
                .ok_or_else(|| Error("polling-rate needs 1000, 500, 250 or 125".into()))?;
            Device::open()?.set_polling_rate(hz)?;
            store.record_polling_rate(hz)?;
        }
        Some("sensitivity") => {
            let n = |i: usize| args.get(i).and_then(|a| a.parse::<u8>().ok());
            let x = n(1).ok_or_else(|| Error("sensitivity needs <x> [y], each 1-10".into()))?;
            Device::open()?.set_sensitivity(x, n(2).unwrap_or(x))?;
        }
        Some("rescue") => {
            let dev = Device::open()?;
            dev.set_normal_mode()?;
            let max = lindwurm_core::config::SENSITIVITY_MAX;
            dev.set_sensitivity(max, max)?;
            println!("mouse set to normal mode and full speed");
        }
        Some("buttons") => {
            for (name, _, default) in BUTTONS {
                let aliases: Vec<&str> = BUTTON_ALIASES
                    .iter()
                    .filter(|(_, c)| c == name)
                    .map(|(a, _)| *a)
                    .collect();
                let aka = if aliases.is_empty() {
                    String::new()
                } else {
                    format!("  (also: {})", aliases.join(", "))
                };
                println!("{name:<13} factory {default}{aka}");
            }
        }
        Some("actions") => print!("{}", action_help()),
        Some("udev-rule") => print!("{}", lindwurm_core::device::UDEV_RULE),
        Some(other) => return Err(Error(format!("unknown command {other:?}"))),
    }
    Ok(())
}

fn main() -> ExitCode {
    // println! panics on a closed pipe (`lindwurm actions | head`); end quietly instead.
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        eprintln!("{msg}");
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lindwurm: {e}");
            ExitCode::FAILURE
        }
    }
}
