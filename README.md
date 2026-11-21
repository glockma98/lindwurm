# lindwurm

Configure buttons, macros, DPI and LED of gaming mice built on the Holtek
`04d9:a067` controller, such as the **Sharkoon Drakonia Black**, without the
vendor's Windows software. lindwurm comes as a command line tool, a terminal
interface and a small graphical interface; all three share the same profiles.

The settings are stored in the mouse itself, so a configured mouse keeps its
layout on any computer.

> lindwurm is an independent project and not affiliated with or endorsed by
> Sharkoon, Perixx or Holtek. Product names are used only to describe which
> hardware is supported.

## Supported hardware

| Device | USB ID | Status |
|---|---|---|
| Sharkoon Drakonia Black | `04d9:a067` | buttons, macros, DPI and LED tested on Linux and Windows 11 |
| Sharkoon Drakonia | `04d9:a067` | same vendor software, expected to work |
| Perixx MX-2000 / MX-3000 | `04d9:a067` | same controller, untested |

Other Holtek mice use different IDs and protocols and are not supported.

| Platform | Status |
|---|---|
| Linux | tested |
| Windows 11 | tested |
| macOS | builds in CI, not yet tested with a mouse |

## Install

Download the archive for your system from the
[releases](../../releases) and unpack it anywhere. `lindwurm-gui` starts
without a terminal; `lindwurm` and `lindwurm-tui` are run from a terminal.

To build from source instead, install [Rust](https://rustup.rs) and run:

```sh
cargo install --git https://github.com/glockma98/lindwurm lindwurm lindwurm-tui lindwurm-gui
```

On Linux this also needs `pkg-config` and the libudev development files
(package names vary by distribution).

### Linux: access to the mouse

Regular users cannot talk to the mouse until a udev rule grants access:

```sh
lindwurm udev-rule | sudo tee /etc/udev/rules.d/70-lindwurm.rules
sudo udevadm control --reload-rules
```

Then unplug and replug the mouse. The rule is also in
[`contrib/70-lindwurm.rules`](contrib/70-lindwurm.rules).

### Windows and macOS

No driver is needed. The release binaries are not signed, so the system may
warn when they are started for the first time. macOS may ask for the *Input
Monitoring* permission.

If the vendor software is installed, close it while writing to the mouse.

## Quick start

```sh
lindwurm show factory              # the layout the mouse ships with
lindwurm clone factory default     # your own profile to edit
lindwurm set default buttons.illumination consumer:calculator
lindwurm apply                     # writes "default" (or "factory" if missing)
```

Or start `lindwurm-gui` / `lindwurm-tui`, pick a profile, edit it and apply it.
`lindwurm apply factory` restores the factory layout at any time.

## Interfaces

**GUI** (`lindwurm-gui`): profiles on the left with *Apply*, *Clone*, *Delete*
and *Import Mouse.ini* (also takes `.conf` exports); every setting of the
selected profile on the right, with menus for actions, keys, consumer keys
and macros, and a color picker for the LED. Each field shows whether its
value is set in the profile, inherited, or the factory default.

**TUI** (`lindwurm-tui`): the same in a terminal. `Tab` switches between the
profile list and the fields. On a field, `Enter` opens a picker (type to
filter) or, for macros, the macro builder; `e` edits the raw text, `x` resets
it. In the profile list: `Enter` apply, `c` clone, `D` delete, `i` import,
`1`-`7` set the DPI level.

**CLI** (`lindwurm`):

| Command | |
|---|---|
| `apply [profile]` | write a profile to the mouse |
| `show [profile]` | show all settings of a profile |
| `check [profile]` | validate and print the raw bytes, write nothing |
| `profiles` | list profiles, `*` marks the one written last |
| `status` | the profile written last and its bytes |
| `set <profile> <field> [value]` | change a field; without value it is reset |
| `clone <from> <to>` / `delete <profile>` | manage profiles |
| `import <file> [prefix]` | convert the vendor software's Mouse.ini or exported `.conf` |
| `dpi <1-7>` | set the DPI level now |
| `buttons` / `actions` | list button names and actions |
| `udev-rule` | print the Linux udev rule |
| `version` | print the version |

Switching profiles from a hotkey is a matter of running `lindwurm apply <name>`.

## Profiles

Profiles are TOML files in `~/.config/lindwurm/profiles/` on Linux and
`%APPDATA%\lindwurm\config\profiles\` on Windows; `lindwurm profiles` prints
the folder on every system. The built-in profile `factory` holds the factory
settings and is used when there is no `default` profile. A profile can build
on another one with `extends` and only list what differs:

```toml
extends = "factory"
dpi = 5

[buttons]
illumination = "macro:reload"     # the Illumination Key, also called side-right
thumb-bottom = "consumer:browser-back"
tilt-left    = "key:pageup"

[macros.reload]
steps = ["r", "100ms", "r"]

[led]
color = "#ff8000"
```

More examples are in [`examples/`](examples).

### Buttons

`left`, `right`, `middle`, `thumb-front`, `thumb-back`, `thumb-bottom`,
`tilt-left`, `tilt-right`, `dpi-up`, `dpi-down`, `side-right` (alias
`illumination`), `wheel-up`, `wheel-down`.

### Actions

- Built-in: `left-click`, `right-click`, `middle-click`, `back`, `forward`,
  `dpi-up`, `dpi-down`, `led-switch`, `double-click`, media keys, `none`, and
  the vendor software's shortcuts (`copy`, `paste`, `lock-pc`, …).
  The vendor's names (`click`, `menu`, `disable`, …) work as well.
- `key:<name>`: a single key, e.g. `key:g`, `key:f5`, `key:0x0a`.
- `consumer:<name>`: media and system keys, e.g. `consumer:calculator`, or any
  HID consumer usage as `consumer:0x0192`.
- `macro:<name>`: a macro from the `[macros]` table.
- `raw:<8 hex digits>`: a raw button slot, for experiments.

`lindwurm actions` lists everything.

### Macros

Steps are `"+k"` (press), `"-k"` (release), `"k"` (press and release) and
pauses like `"150ms"` (multiples of 50 ms, up to 6350 ms after a step). Mouse
buttons are `lmb`, `rmb`, `mmb`, `mb4`, `mb5`. A macro holds up to 47 steps;
since every pressed key has to be released, 46 steps (23 key taps) is the
practical limit. Up to 16 macros fit on the mouse; `repeat = N` repeats a
macro up to 32767 times. Key names follow US key positions, so on other
layouts some keys are swapped (e.g. `y` and `z` on German keyboards).

### Importing the vendor software's settings

`lindwurm import Mouse.ini` turns the five profiles of the Windows software
(`%APPDATA%\MingGuan\GamingMouse\Mouse.ini`) into profiles `ini-1` to
`ini-5`. Every converted entry keeps a comment with the original value;
anything that cannot be converted is reported and left as a comment.
Profiles saved with the vendor software's export button (`*.conf`, by default
`profile.conf`) are imported the same way as a single profile.

## Limitations

- The mouse cannot be read back. lindwurm remembers what it wrote last
  (`lindwurm status`); changes made elsewhere are not visible.
- Keys above HID usage `0x65` (e.g. F13–F24) are sent by the mouse but
  dropped by Linux, because the mouse declares a smaller key range.
- `scroll-left`, `scroll-right` and `profile-switch` (the factory setting of
  the wheel tilt) are only reports to the computer that the vendor's Windows
  service turned into actions; without it they do nothing.
- Polling rate and sensitivity are known but not implemented yet.
- Writing a profile rewrites the button table and all macro slots in the
  mouse's memory. Interrupted writes have so far always been fixed by
  replugging and writing again, but use it at your own risk.

## How it works

The mouse takes 8-byte HID feature reports (ID 7) and output reports, each
scrambled with a fixed key. The protocol was first worked out for the Perixx
MX-3000 by [mx3000control](https://github.com/gianluca-nitti/mx3000control);
lindwurm confirms it against the Drakonia Black's own software and adds the
button map, macro limits, consumer keys, LED and the vendor's profile format.

## License

MIT, see [LICENSE](LICENSE). The scrambling code is derived from
mx3000control, see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
