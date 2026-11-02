//! The live data behind the builtin screens.
//!
//! Each provider is split in two: a pure parser (tested here) and a thin
//! wrapper that actually runs the command or walks the filesystem.

use pt35_common::apps::{initials, pretty_app};
use pt35_common::menu::{Builtin, Layout};
use pt35_common::theme::Rgb;
use std::process::Command;

/// One row of a dynamic screen. `payload` is opaque to the model: only the
/// provider and the code that acts on the row know what it means.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    pub label: String,
    pub payload: String,
    /// Second line on a tile, right-hand text on a row.
    pub note: String,
    /// One or two characters standing in for an icon.
    pub glyph: String,
    /// freedesktop icon name, preferred over the glyph.
    pub icon: String,
    /// The one row that is current: the network you are on, the accent in use.
    pub active: bool,
    /// A colour swatch drawn in place of the icon.
    pub tint: Option<Rgb>,
    /// What a search matches besides the label. Empty means the payload.
    pub search: String,
}

impl Item {
    pub fn new(label: impl Into<String>, payload: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            payload: payload.into(),
            ..Self::default()
        }
    }
}

pub type Items = Vec<Item>;

/// Everything the shell needs to know about one builtin screen.
///
/// One table rather than a `match Builtin` per question. Adding a screen is a
/// row here plus a function to fill it, and nothing can be half-added.
pub struct Screen {
    pub title: &'static str,
    /// A grid when you recognise a row by its shape faster than you read it.
    pub layout: Layout,
    /// What to say while the rows are still being fetched, when fetching them
    /// is slow enough to need saying. A Wi-Fi scan is seconds of `nmcli`.
    pub scanning: Option<&'static str>,
    /// Where the rows come from.
    pub rows: fn() -> Items,
    /// Rows you read, not press. No cursor, no numbers, no A in the legend.
    pub readonly: bool,
}

pub fn screen(builtin: Builtin) -> Screen {
    let (title, layout, scanning, rows): (_, _, _, fn() -> Items) = match builtin {
        Builtin::Launcher => ("Launcher", Layout::List, None, launcher as fn() -> Items),
        Builtin::Quick => ("Quick settings", Layout::List, Some("Reading..."), quick),
        Builtin::System => ("System", Layout::List, None, system),
        // One column: the D-pad only goes up and down, and a whole title fits.
        Builtin::Windows => ("Windows", Layout::List, None, windows),
        Builtin::Wifi => ("Network", Layout::List, Some("Scanning..."), wifi),
        Builtin::Bluetooth => (
            "Bluetooth",
            Layout::List,
            Some("Looking for devices..."),
            bluetooth,
        ),
        Builtin::Audio => ("Volume", Layout::List, None, volume_levels),
        Builtin::Display => ("Brightness", Layout::List, None, brightness_levels),
        Builtin::DesktopEntries => ("All apps", Layout::List, None, desktop_entries),
        Builtin::Appearance => ("Appearance", Layout::List, None, appearance),
        Builtin::About => ("About", Layout::List, None, about),
    };
    Screen {
        title,
        layout,
        scanning,
        rows,
        readonly: matches!(builtin, Builtin::System | Builtin::About),
    }
}

pub fn title(builtin: Builtin) -> &'static str {
    screen(builtin).title
}

pub fn items(builtin: Builtin) -> Items {
    (screen(builtin).rows)()
}

/// The single row shown while a slow screen is still being read.
pub fn placeholder(text: &str) -> Items {
    vec![Item {
        label: text.to_string(),
        payload: String::new(),
        note: String::new(),
        glyph: "read".into(),
        ..Item::default()
    }]
}

fn volume_levels() -> Items {
    levels("volume")
}

fn brightness_levels() -> Items {
    levels("brightness")
}

// --------------------------------------------------------------- launcher

/// Every app: the ones with a profile first, then everything else with a
/// .desktop file. One list, so typing finds any of it. Windows, Settings and
/// Power are not apps and live in the launcher's side column.
///
/// The payload says what activating a row means: `app:` goes through the
/// daemon with its profile, `exec:` is a plain command line.
fn launcher() -> Items {
    let mut out: Items = Vec::new();
    let apps: pt35_common::apps::AppTable = pt35_common::load_config("pt35/apps.toml");
    let mut programs: Vec<String> = Vec::new();
    for (id, app) in &apps.apps {
        // The window it makes and the program it is about: "foot" finds
        // Terminal, "yazi" finds Files. Not every program it runs, or "foot"
        // would find every app that runs in a terminal.
        let window = [&app.match_.app_id, &app.match_.class]
            .into_iter()
            .flatten()
            .map(|name| name.to_lowercase());
        let search: Vec<String> = window.chain(program_of(&app.exec)).collect();
        out.push(Item {
            label: if app.label.is_empty() {
                pretty_app(id)
            } else {
                app.label.clone()
            },
            payload: format!("app:{id}"),
            note: String::new(),
            glyph: app.glyph.clone(),
            icon: app.icon.clone(),
            search: format!("{id} {}", search.join(" ")),
            ..Item::default()
        });
        // Every program it runs, for telling a .desktop copy of it: Terminal is
        // `foot pt35-hello`, and Foot is it. Not a program that only starts
        // others: a profile run through flatpak would hide every flatpak.
        programs.extend(
            programs_of(&app.exec)
                .into_iter()
                .filter(|p| !LAUNCHERS.contains(&p.as_str())),
        );
    }

    let theme = pt35_common::load_theme();
    // Anything else with a .desktop file, so search covers the whole machine.
    // Not a second copy of a profiled app: "Mousepad" is the Editor, and its
    // .desktop copy would launch without the profile's scale and flags.
    if theme.menu.show_desktop_entries {
        let known: Vec<String> = out.iter().map(|i| i.label.to_lowercase()).collect();
        for mut entry in desktop_entries() {
            let program = program_of(&entry.payload);
            let profiled = program.as_ref().is_some_and(|p| programs.contains(p));
            if !profiled && !known.contains(&entry.label.to_lowercase()) {
                entry.search = program.unwrap_or_default();
                entry.payload = format!("exec:{}", entry.payload);
                out.push(entry);
            }
        }
    }
    let mut recent = recents();
    recent.truncate(theme.menu.recents as usize);
    promote_recent(&mut out, &recent);
    // Pins above the recents: they are the ones you said you always want.
    let pinned = pins();
    promote_recent(&mut out, &pinned);
    for item in &mut out {
        item.active = pinned.contains(&item.payload);
    }
    out
}

/// Programs that start other programs, and so say nothing about which app it is.
const LAUNCHERS: &[&str] = &[
    "env", "flatpak", "python", "python3", "sudo", "pkexec", "nice",
];

/// The programs a command line runs, by file name, lowercase: for a terminal,
/// the terminal and then what runs in it.
fn programs_of(exec: &str) -> Vec<String> {
    pt35_common::apps::command_programs(exec)
        .iter()
        .map(|program| program.rsplit('/').next().unwrap_or(program).to_lowercase())
        .collect()
}

/// The program a command line is really about: for a terminal, the one in it.
fn program_of(exec: &str) -> Option<String> {
    programs_of(exec).pop()
}

/// What you pinned, first pinned first.
pub fn pins() -> Vec<String> {
    read_pins().unwrap_or_default()
}

fn read_pins() -> std::io::Result<Vec<String>> {
    match std::fs::read_to_string(pt35_common::paths::pins_path()) {
        Ok(text) => Ok(parse_list(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

fn parse_list(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim) {
        if !line.is_empty() && !out.iter().any(|seen| seen == line) {
            out.push(line.to_string());
        }
    }
    out
}

/// Pin a launcher row, or unpin it if it already is. Returns whether it is
/// pinned now.
pub fn toggle_pin(payload: &str) -> std::io::Result<bool> {
    // A file that cannot be read is not an empty one: writing the new pin alone
    // would drop every other.
    let mut list = read_pins()?;
    let pinned = if let Some(at) = list.iter().position(|p| p == payload) {
        list.remove(at);
        false
    } else {
        list.push(payload.to_string());
        true
    };
    pt35_common::write_atomic(
        &pt35_common::paths::pins_path(),
        (list.join("\n") + "\n").as_bytes(),
    )?;
    Ok(pinned)
}

/// Move the things you opened last to the top, in that order.
///
/// `AppTable` is a `BTreeMap`, so the launcher's own order is by app id: one
/// nobody chose and nobody remembers.
pub fn promote_recent(items: &mut Items, recent: &[String]) {
    // Walk the recents backwards so the newest ends up at index 0.
    for payload in recent.iter().rev() {
        if let Some(at) = items.iter().position(|item| &item.payload == payload) {
            let item = items.remove(at);
            items.insert(0, item);
        }
    }
}

fn recents() -> Vec<String> {
    std::fs::read_to_string(pt35_common::paths::recents_path())
        .map(|text| parse_list(&text))
        .unwrap_or_default()
}

// ----------------------------------------------------------------- system

/// The dashboard. Everything here is a file in /proc or /sys, read once when
/// the screen opens: no daemon round trip, no sampling thread.
pub fn system() -> Items {
    let mut out = Vec::new();
    if let Some((used, total)) = memory() {
        out.push(bar_item(
            "Memory",
            used * 100 / total.max(1),
            &format!(
                "{:.1} / {:.1} GB",
                used as f32 / 1024.0 / 1024.0,
                total as f32 / 1024.0 / 1024.0
            ),
            "memory",
        ));
    }
    if let Some((used, total)) = storage() {
        out.push(bar_item(
            "Storage",
            used * 100 / total.max(1),
            &format!("{used} / {total} GB"),
            "drive-multidisk",
        ));
    }
    if let Some(load) = load_percent() {
        out.push(bar_item(
            "Load",
            load,
            &format!("{load}%"),
            "utilities-system-monitor",
        ));
    }
    if let Some(temp) = temperature() {
        out.push(read_item("Temperature", &format!("{temp:.1} °C"), "temp"));
    }
    if let Some(address) = ip_address() {
        out.push(read_item("Address", &address, "network-wired"));
    }
    if let Some(up) = uptime() {
        out.push(read_item("Uptime", &up, "clock"));
    }
    out
}

fn bar_item(label: &str, percent: u64, note: &str, icon: &str) -> Item {
    Item {
        label: label.into(),
        payload: String::new(),
        note: note.into(),
        glyph: format!("bar:{}", percent.min(100)),
        icon: icon.into(),
        ..Item::default()
    }
}

fn read_item(label: &str, note: &str, icon: &str) -> Item {
    Item {
        label: label.into(),
        payload: String::new(),
        note: note.into(),
        glyph: "read".into(),
        icon: icon.into(),
        ..Item::default()
    }
}

/// Used and total, in kB, the way `free` counts it.
fn memory() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(name))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    Some((total.saturating_sub(available), total))
}

/// Used and total of the root filesystem, in GB.
fn storage() -> Option<(u64, u64)> {
    let out = run("df", &["-BG", "--output=used,size", "/"])?;
    let line = out.lines().nth(1)?;
    let mut fields = line.split_whitespace();
    let used: u64 = fields.next()?.trim_end_matches('G').parse().ok()?;
    let size: u64 = fields.next()?.trim_end_matches('G').parse().ok()?;
    Some((used, size))
}

/// One-minute load as a percentage of all cores.
fn load_percent() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/loadavg").ok()?;
    let one: f32 = text.split_whitespace().next()?.parse().ok()?;
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1) as f32;
    Some(((one / cores) * 100.0).min(100.0) as u64)
}

fn temperature() -> Option<f32> {
    let raw = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").ok()?;
    Some(raw.trim().parse::<f32>().ok()? / 1000.0)
}

fn ip_address() -> Option<String> {
    let out = run("hostname", &["-I"])?;
    out.split_whitespace().next().map(str::to_string)
}

fn uptime() -> Option<String> {
    let text = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds: f64 = text.split_whitespace().next()?.parse().ok()?;
    Some(duration(seconds as u64))
}

/// `3d 4h`, `4h 12m`, `12m`: the two units that matter, not a stopwatch.
pub fn duration(seconds: u64) -> String {
    let (days, hours, minutes) = (seconds / 86400, seconds % 86400 / 3600, seconds % 3600 / 60);
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

// ------------------------------------------------------------------ quick

/// The quick panel: switches with their state, sliders with their value, and
/// three ways out to the screens behind them.
///
/// `note` is the right-hand readout and `glyph` says how to draw the row:
/// `switch`, `slide`, `bar`, `nav` or `read`.
pub fn quick() -> Items {
    let status = crate::live::status();
    let signal = status.as_ref().and_then(|s| s.network_signal);
    let ethernet = status.as_ref().and_then(|s| s.ethernet.clone());
    let volume = status.as_ref().and_then(|s| s.volume_percent).unwrap_or(0);
    let muted = status.as_ref().and_then(|s| s.muted).unwrap_or(false);
    let mouse = status
        .as_ref()
        .map(|s| s.input_mode == pt35_common::ipc::InputMode::Mouse)
        .unwrap_or(false);
    let touch = status.as_ref().map(|s| s.touch_enabled).unwrap_or(true);
    let brightness = status.as_ref().and_then(|s| s.brightness_percent);

    // The radio, as the toggle reads it. A signal says only that it is joined:
    // on but not connected read as off, and the toggle then turned it off.
    let wifi_on = wifi_radio().unwrap_or(signal.is_some());
    let mut rows = vec![Item {
        label: "Wi-Fi".into(),
        payload: "sh:pt35-quick wifi toggle".into(),
        note: match (wifi_on, wifi_name()) {
            (true, Some(ssid)) => ssid,
            (true, None) if signal.is_some() => "on".into(),
            (true, None) => "not connected".into(),
            (false, _) => String::new(),
        },
        glyph: switch(wifi_on),
        icon: pt35_ui::icon::wifi_icon(signal).into(),
        ..Item::default()
    }];
    // Only with a cable in: a row that can only read "unplugged" is noise.
    rows.extend(ethernet.map(|iface| Item {
        glyph: "read".into(),
        active: false,
        ..ethernet_row(&iface)
    }));
    rows.extend([
        Item {
            label: "Bluetooth".into(),
            payload: "sh:pt35-quick bluetooth toggle".into(),
            note: bluetooth_state(),
            glyph: switch(bluetooth_on()),
            icon: "bluetooth".into(),
            ..Item::default()
        },
        Item {
            label: "Input".into(),
            payload: "ctl:mode toggle".into(),
            note: if mouse { "MOUSE" } else { "BUTTONS" }.into(),
            // Not a switch: neither mode is "off".
            glyph: "read".into(),
            icon: if mouse {
                "input-mouse"
            } else {
                "input-keyboard"
            }
            .into(),
            ..Item::default()
        },
        Item {
            label: "Touch".into(),
            payload: "ctl:touch toggle".into(),
            note: String::new(),
            glyph: switch(touch),
            icon: "input-touchpad".into(),
            ..Item::default()
        },
        volume_row(volume, muted),
        brightness_row(brightness),
        Item {
            label: "Networks".into(),
            payload: "screen:wifi".into(),
            note: String::new(),
            glyph: "nav".into(),
            icon: "network-wireless".into(),
            ..Item::default()
        },
        Item {
            label: "Settings".into(),
            payload: "page:settings".into(),
            note: String::new(),
            glyph: "nav".into(),
            icon: "preferences-system".into(),
            ..Item::default()
        },
    ]);
    rows
}

/// The cable, read-only: NetworkManager brings it up on its own, and there
/// is nothing to choose.
fn ethernet_row(iface: &str) -> Item {
    Item {
        label: "Ethernet".into(),
        payload: String::new(),
        note: address_of(iface).unwrap_or_else(|| "no address".into()),
        icon: "network-wired".into(),
        active: true,
        ..Item::default()
    }
}

fn address_of(iface: &str) -> Option<String> {
    let out = run("ip", &["-4", "-o", "addr", "show", "dev", iface])?;
    parse_address(&out)
}

/// `ip -o` prints `3: eth0    inet 192.168.1.20/24 brd ...`.
pub fn parse_address(out: &str) -> Option<String> {
    let mut words = out.split_whitespace();
    words.find(|w| *w == "inet")?;
    let cidr = words.next()?;
    Some(cidr.split('/').next()?.to_string())
}

fn volume_row(volume: u8, muted: bool) -> Item {
    Item {
        label: "Volume".into(),
        payload: "adjust:volume".into(),
        note: if muted {
            "muted".into()
        } else {
            format!("{volume}%")
        },
        glyph: format!("slide:{}", if muted { 0 } else { volume }),
        icon: pt35_ui::icon::volume_icon(volume, muted).into(),
        ..Item::default()
    }
}

const BRIGHTNESS: &str = "Brightness";

fn brightness_row(level: Option<u8>) -> Item {
    match level {
        // The keyboard's firmware answers: a slider like volume.
        Some(level) => Item {
            label: BRIGHTNESS.into(),
            payload: "adjust:brightness".into(),
            note: format!("{level}%"),
            glyph: format!("slide:{level}"),
            icon: "display-brightness".into(),
            ..Item::default()
        },
        // Stock firmware keeps the PWM pin to itself: say which keys do it.
        None => Item {
            label: BRIGHTNESS.into(),
            payload: String::new(),
            note: "Fn  -  =".into(),
            glyph: "read".into(),
            icon: "display-brightness".into(),
            ..Item::default()
        },
    }
}

/// Move the quick panel's sliders to a fresh status, in place. A step has to
/// show at once: reading the whole panel again forks nmcli and bluetoothctl
/// and lands a press or two behind.
pub fn patch_levels(items: &mut Items, status: &pt35_common::ipc::Status) {
    for item in items.iter_mut() {
        if item.payload == "adjust:volume" {
            *item = volume_row(
                status.volume_percent.unwrap_or(0),
                status.muted.unwrap_or(false),
            );
        } else if item.label == BRIGHTNESS {
            *item = brightness_row(status.brightness_percent);
        }
    }
}

fn switch(on: bool) -> String {
    if on { "switch:on" } else { "switch:off" }.to_string()
}

/// The network you are on, which is worth more than the interface name.
/// From nmcli, which the Network screen needs anyway: iwgetid is not installed
/// on Raspberry Pi OS Lite. The SSID, not the connection's name: Pi Imager
/// calls every one it makes "preconfigured". From the last scan, not a new one.
fn wifi_name() -> Option<String> {
    let out = run(
        "nmcli",
        &[
            "-t",
            "-f",
            "ACTIVE,SSID",
            "device",
            "wifi",
            "list",
            "--rescan",
            "no",
        ],
    )?;
    active_wifi(&out)
}

fn wifi_radio() -> Option<bool> {
    run("nmcli", &["radio", "wifi"]).map(|out| out.trim() == "enabled")
}

/// `ACTIVE:SSID` lines; the one marked `yes` is the network you are on.
pub fn active_wifi(out: &str) -> Option<String> {
    out.lines().find_map(|line| {
        let fields = nmcli_fields(line);
        (fields.first()? == "yes")
            .then(|| fields.get(1).cloned())
            .flatten()
            .filter(|ssid| !ssid.is_empty())
    })
}

/// One line of nmcli's terse output, split into fields. A `:` inside a field
/// comes escaped as `\:`, and a backslash as `\\`.
pub fn nmcli_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        let c = match c {
            '\\' => match chars.next() {
                Some(next) => next,
                None => break,
            },
            ':' => {
                fields.push(String::new());
                continue;
            }
            c => c,
        };
        if let Some(field) = fields.last_mut() {
            field.push(c);
        }
    }
    fields
}

fn bluetooth_on() -> bool {
    run("bluetoothctl", &["show"])
        .map(|out| out.contains("Powered: yes"))
        .unwrap_or(false)
}

fn bluetooth_state() -> String {
    let Some(out) = run("bluetoothctl", &["devices", "Connected"]) else {
        return String::new();
    };
    match out.lines().filter(|l| l.starts_with("Device ")).count() {
        0 => String::new(),
        1 => "1 device".into(),
        n => format!("{n} devices"),
    }
}

// ---------------------------------------------------------------- windows

/// The picker. The daemon already knows every window and which profile owns it,
/// so the glyph comes back with the list instead of being guessed here.
/// Payload of the picker's Launcher tile. It is not a window: it cannot be
/// closed, and picking it goes to the launcher.
pub const HOME: &str = "home";

/// An icon name no theme has: the tile draws the pixel skull for it.
pub const SKULL_ICON: &str = "pt35:skull";

fn windows() -> Items {
    // The launcher is always there to switch to, even with nothing open.
    let home = Item {
        label: "Launcher".into(),
        payload: HOME.into(),
        note: "apps and settings".into(),
        glyph: "::".into(),
        // Drawn by the tile itself, the same mark as the bar's menu button.
        icon: SKULL_ICON.into(),
        ..Item::default()
    };
    let Some(status) = crate::live::status() else {
        return vec![home];
    };
    // The menu holds the keyboard, so sway reports no window focused. The one
    // you came from is the tiled window on the current workspace.
    let current = status
        .windows
        .iter()
        .find(|w| w.focused)
        .or_else(|| {
            status
                .windows
                .iter()
                .find(|w| w.workspace == status.workspace && !w.floating)
        })
        .map(|w| w.id);
    std::iter::once(home)
        .chain(status.windows.iter().map(|w| Item {
            label: pretty_app(&w.app),
            payload: format!("con:{}", w.id),
            // "Foot / foot" says one thing twice.
            note: if w.title.eq_ignore_ascii_case(&pretty_app(&w.app)) {
                String::new()
            } else {
                w.title.clone()
            },
            glyph: if w.glyph.is_empty() {
                initials(&w.app)
            } else {
                w.glyph.clone()
            },
            // No profile: an icon named after the app is right more often
            // than not, and the tile falls back to the generic one.
            icon: if w.icon.is_empty() {
                w.app.to_lowercase()
            } else {
                w.icon.clone()
            },
            active: Some(w.id) == current,
            ..Item::default()
        }))
        .collect()
}

// ------------------------------------------------------------------- wifi

/// The cable first when one is in, then the Wi-Fi networks in range.
fn wifi() -> Items {
    let ethernet = crate::live::status().and_then(|s| s.ethernet);
    let mut rows: Items = ethernet
        .map(|iface| ethernet_row(&iface))
        .into_iter()
        .collect();
    rows.extend(
        match run(
            "nmcli",
            &["-t", "-f", "ACTIVE,SIGNAL,SSID", "device", "wifi", "list"],
        ) {
            Some(out) => parse_nmcli(&out),
            None => vec![Item::new("nmcli not installed", "")],
        },
    );
    rows
}

/// nmcli terse output: `ACTIVE:SIGNAL:SSID`, strongest first, no duplicates.
pub fn parse_nmcli(out: &str) -> Items {
    let mut seen: Vec<(String, i32, bool)> = Vec::new();
    for line in out.lines() {
        let fields = nmcli_fields(line);
        let field = |at: usize| fields.get(at).map(String::as_str).unwrap_or("");
        let active = field(0) == "yes";
        let signal: i32 = field(1).parse().unwrap_or(0);
        // Not trimmed: a leading or trailing space is part of the name you join.
        let ssid = field(2).to_string();
        if ssid.is_empty() {
            continue;
        }
        match seen.iter_mut().find(|(name, ..)| *name == ssid) {
            Some(entry) => {
                entry.1 = entry.1.max(signal);
                entry.2 |= active;
            }
            None => seen.push((ssid, signal, active)),
        }
    }
    // The network you are on first, then by strength.
    seen.sort_by_key(|entry| (!entry.2, std::cmp::Reverse(entry.1)));
    seen.into_iter()
        .map(|(ssid, signal, active)| Item {
            label: ssid.clone(),
            payload: ssid,
            note: if active {
                format!("connected  {signal}%")
            } else {
                format!("{signal}%")
            },
            icon: pt35_ui::icon::wifi_icon(Some(signal.clamp(0, 100) as u8)).into(),
            active,
            ..Item::default()
        })
        .collect()
}

// -------------------------------------------------------------- bluetooth

fn bluetooth() -> Items {
    let Some(out) = run("bluetoothctl", &["devices"]) else {
        return vec![Item::new("bluetoothctl not installed", "")];
    };
    // An empty list with the radio off reads as "no devices", which is not
    // the problem. Say what is, and let A fix it.
    let show = run("bluetoothctl", &["show"]).unwrap_or_default();
    if !show.contains("Powered: yes") {
        return vec![Item {
            label: "Bluetooth is off".into(),
            payload: "sh:pt35-quick bluetooth on".into(),
            note: "A to turn on".into(),
            icon: "bluetooth".into(),
            ..Item::default()
        }];
    }
    let connected = run("bluetoothctl", &["devices", "Connected"]).unwrap_or_default();
    let paired = run("bluetoothctl", &["devices", "Paired"]).unwrap_or_default();
    let mut items = parse_bluetoothctl(&out);
    for item in &mut items {
        item.icon = "bluetooth".into();
        let mac = item.payload.clone();
        if connected.contains(&mac) {
            item.active = true;
            item.note = "connected".into();
            // A on a connected device lets it go.
            item.payload = format!("sh:pt35-quick bluetooth disconnect {mac}");
        } else if paired.contains(&mac) {
            item.note = "paired".into();
        } else {
            item.note = "new".into();
        }
    }
    // Connected, then paired, then what the scan found, named ones first: a
    // device with no name shows its address, and there are dozens nearby.
    items.sort_by_key(|item| {
        let rank = match item.note.as_str() {
            "connected" => 0,
            "paired" => 1,
            _ => 2,
        };
        (rank, is_unnamed(&item.label))
    });
    let scanning = show.contains("Discovering: yes");
    items.insert(
        0,
        Item {
            label: if scanning {
                "Scanning...".into()
            } else {
                "Scan for devices".into()
            },
            payload: if scanning {
                String::new()
            } else {
                "sh:pt35-quick bluetooth scan".into()
            },
            note: if scanning { "12 s" } else { "" }.into(),
            icon: "view-refresh".into(),
            ..Item::default()
        },
    );
    items
}

/// bluetoothctl names a device with no name after its address, dashed.
pub fn is_unnamed(label: &str) -> bool {
    label.len() == 17
        && label.split('-').count() == 6
        && label
            .split('-')
            .all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `Device AA:BB:CC:DD:EE:FF Name Of Thing`
pub fn parse_bluetoothctl(out: &str) -> Items {
    out.lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Device ")?;
            let (mac, name) = rest.split_once(' ')?;
            Some(Item::new(name.trim(), mac))
        })
        .collect()
}

// ----------------------------------------------------------------- levels

/// Volume and brightness screens are just a ladder of absolute values: a slider
/// is useless without a pointer, a list of ten steps is two key presses.
fn levels(what: &str) -> Items {
    (0..=10)
        .rev()
        .map(|step| {
            let percent = step * 10;
            Item::new(format!("{percent:>3}%"), format!("{what} {percent}"))
        })
        .collect()
}

// --------------------------------------------------------- desktop entries

fn desktop_entries() -> Items {
    let desktops: Vec<String> = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .map(str::to_lowercase)
        .collect();
    let desktops: Vec<&str> = desktops.iter().map(String::as_str).collect();
    // The first file with a name wins, so the user's copy overrides the
    // system's, and a copy with Hidden=true hides it. That is the spec's way.
    let mut seen = std::collections::HashSet::new();
    let mut out: Items = Vec::new();
    for dir in application_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<std::path::PathBuf> = entries.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_name().map(|n| n.to_os_string()) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(item) = parse_entry(&text, &desktops, &pt35_common::apps::on_path) {
                out.push(item);
            }
        }
    }
    out.sort_by_key(|entry| entry.label.to_lowercase());
    out.dedup_by(|a, b| a.label == b.label);
    out
}

/// Where .desktop files live, most important first.
fn application_dirs() -> Vec<std::path::PathBuf> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let data_home = var("XDG_DATA_HOME").or_else(|| var("HOME").map(|h| h + "/.local/share"));
    let data_dirs = var("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    for dir in data_home
        .iter()
        .map(String::as_str)
        .chain(data_dirs.split(':'))
    {
        let dir = std::path::Path::new(dir).join("applications");
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

/// Pull `Name` and `Exec` out of a .desktop file, skipping hidden ones and
/// stripping the field codes (`%U`, `%f`, …) that would confuse `sh -c`.
#[cfg(test)]
pub fn parse_desktop_entry(text: &str) -> Option<Item> {
    parse_entry(text, &[], &|_| true)
}

/// `desktops` is `$XDG_CURRENT_DESKTOP`, lowercase, for OnlyShowIn and
/// NotShowIn. `installed` answers TryExec.
fn parse_entry(text: &str, desktops: &[&str], installed: &dyn Fn(&str) -> bool) -> Option<Item> {
    let mut name = None;
    let mut exec = None;
    let mut icon = None;
    let mut terminal = false;
    let mut in_main_section = false;
    let shown_in = |list: &str| {
        list.split(';')
            .any(|d| !d.is_empty() && desktops.contains(&d.to_lowercase().as_str()))
    };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main_section = line == "[Desktop Entry]";
            continue;
        }
        if !in_main_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unescape_desktop(value.trim());
        match key.trim() {
            "Name" => name = Some(value),
            "Exec" => exec = Some(value),
            "Icon" => icon = Some(value),
            "Terminal" => terminal = value.eq_ignore_ascii_case("true"),
            "NoDisplay" | "Hidden" if value.eq_ignore_ascii_case("true") => return None,
            "Type" if value != "Application" => return None,
            "OnlyShowIn" if !shown_in(&value) => return None,
            "NotShowIn" if shown_in(&value) => return None,
            "TryExec" if !installed(&value) => return None,
            _ => {}
        }
    }
    let exec = strip_field_codes(&exec?);
    if exec.is_empty() {
        return None;
    }
    // `Terminal=true` means a text program, htop or vim: started bare it has
    // nowhere to draw and exits at once.
    let exec = if terminal {
        let id = crate::exec::shell_quote(&format!(
            "pt35-{}",
            pt35_common::apps::command_binary(&exec).unwrap_or_else(|| "term".into())
        ));
        format!("foot -a {id} {exec}")
    } else {
        exec
    };
    let name = name?;
    Some(Item {
        glyph: name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .to_string(),
        // A path instead of a name is a PNG more often than not, and nothing
        // here decodes one: the letter stands in.
        icon: icon.filter(|i| !i.contains('/')).unwrap_or_default(),
        label: name,
        payload: exec,
        ..Item::default()
    })
}

/// The string escapes of a .desktop value: `\s`, `\n`, `\t`, `\r`, `\\`.
/// Any other escape is left for the Exec line's own quoting.
fn unescape_desktop(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// No files are handed over, so a word that carries a field code goes whole:
/// `--file=%f` with no file is worse than no flag. `%%` is a plain `%`.
fn strip_field_codes(exec: &str) -> String {
    const CODES: &str = "fFuUdDnNickvm";
    exec.split_whitespace()
        .filter_map(|word| {
            let mut out = String::new();
            let mut chars = word.chars();
            while let Some(c) = chars.next() {
                if c != '%' {
                    out.push(c);
                    continue;
                }
                match chars.next() {
                    Some('%') => out.push('%'),
                    Some(code) if CODES.contains(code) => return None,
                    Some(other) => {
                        out.push('%');
                        out.push(other);
                    }
                    None => out.push('%'),
                }
            }
            Some(out)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ------------------------------------------------------------------ about

fn about() -> Items {
    let mut out = vec![read_item("Shell", env!("CARGO_PKG_VERSION"), "computer")];
    if let Ok(model) = std::fs::read_to_string("/proc/device-tree/model") {
        let model = model
            .trim_end_matches('\0')
            .trim()
            .replace("Raspberry Pi", "Pi");
        out.push(read_item("Board", &model, "cpu"));
    }
    if let Some(name) = std::fs::read_to_string("/etc/os-release")
        .ok()
        .as_deref()
        .and_then(os_name)
    {
        out.push(read_item("System", &name, "start-here"));
    }
    if let Some(kernel) = run("uname", &["-r"]) {
        out.push(read_item("Kernel", kernel.trim(), "application-x-firmware"));
    }
    if let Ok(host) = std::fs::read_to_string("/etc/hostname") {
        out.push(read_item("Hostname", host.trim(), "network-workgroup"));
    }
    out.push(read_item("Display", &display(), "video-display"));
    out
}

/// `PRETTY_NAME` without its quotes and without the kernel's `GNU/Linux`.
pub fn os_name(release: &str) -> Option<String> {
    let value = release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?
        .trim_matches('"');
    Some(value.replace(" GNU/Linux", ""))
}

/// The output and its mode, from sway rather than assumed.
fn display() -> String {
    let Some(out) = run("swaymsg", &["-t", "get_outputs", "-r"]) else {
        return "640x480".into();
    };
    let outputs: serde_json::Value = serde_json::from_str(&out).unwrap_or_default();
    outputs
        .as_array()
        .and_then(|list| list.iter().find(|o| o["active"].as_bool() == Some(true)))
        .map(|o| {
            format!(
                "{} {}x{}",
                o["name"].as_str().unwrap_or("?"),
                o["current_mode"]["width"],
                o["current_mode"]["height"]
            )
        })
        .unwrap_or_else(|| "640x480".into())
}

// ------------------------------------------------------------- appearance

/// Accents that keep `accent_fg` readable on top of them: every one is light
/// enough for the near-black text a selected row uses.
pub const ACCENTS: &[(&str, &str)] = &[
    ("Mint", "#3ddc97"),
    ("Cyan", "#5fd0e8"),
    ("Amber", "#e8c468"),
    ("Orange", "#f0a04b"),
    ("Violet", "#b89cf2"),
    ("Pink", "#f28cc4"),
];

/// Whole schemes: every colour key, so nothing is left in the last one's tint.
/// Order: background, background_alt, foreground, muted, accent, accent_fg,
/// warning, critical, ok, border, then the four face buttons and the neutral
/// pill.
pub const PALETTES: &[(&str, &[(&str, &str)])] = &[
    (
        "Mint",
        &palette([
            "#0a1310", "#121f1a", "#dcf3e5", "#7aa48e", "#3ddc97", "#06130d", "#e8c468", "#ef6b73",
            "#3ddc97", "#254236", "#3ddc97", "#ef6b73", "#5fd0e8", "#e8c468", "#254236",
        ]),
    ),
    (
        "Nord",
        &palette([
            "#2e3440", "#3b4252", "#eceff4", "#a3adc2", "#88c0d0", "#2e3440", "#ebcb8b", "#bf616a",
            "#a3be8c", "#4c566a", "#a3be8c", "#bf616a", "#88c0d0", "#ebcb8b", "#4c566a",
        ]),
    ),
    (
        "Gruvbox",
        &palette([
            "#1d2021", "#282828", "#ebdbb2", "#a89984", "#fabd2f", "#1d2021", "#fe8019", "#fb4934",
            "#b8bb26", "#504945", "#b8bb26", "#fb4934", "#83a598", "#fabd2f", "#504945",
        ]),
    ),
    (
        "Dracula",
        &palette([
            "#1e1f29", "#282a36", "#f8f8f2", "#a2a9cf", "#bd93f9", "#1e1f29", "#f1fa8c", "#ff5555",
            "#50fa7b", "#44475a", "#50fa7b", "#ff5555", "#8be9fd", "#f1fa8c", "#44475a",
        ]),
    ),
    (
        "Solarized",
        &palette([
            "#002b36", "#073642", "#eee8d5", "#93a1a1", "#2aa198", "#002b36", "#b58900", "#dc322f",
            "#859900", "#0e4b5a", "#859900", "#dc322f", "#268bd2", "#b58900", "#0e4b5a",
        ]),
    ),
    (
        "Paper",
        &palette([
            "#f4f1ea", "#e8e3d8", "#1f2421", "#5e665f", "#1f7a55", "#f4f1ea", "#9a6a00", "#b3363f",
            "#1f7a55", "#c9c2b3", "#1f7a55", "#b3363f", "#1d6f8a", "#9a6a00", "#c9c2b3",
        ]),
    ),
];

const fn palette(hex: [&'static str; 15]) -> [(&'static str, &'static str); 15] {
    const KEYS: [&str; 15] = [
        "background",
        "background_alt",
        "foreground",
        "muted",
        "accent",
        "accent_fg",
        "warning",
        "critical",
        "ok",
        "border",
        "button_a",
        "button_b",
        "button_x",
        "button_y",
        "button_neutral",
    ];
    let mut out = [("", ""); 15];
    let mut i = 0;
    while i < 15 {
        out[i] = (KEYS[i], hex[i]);
        i += 1;
    }
    out
}

const CLOCKS: &[(&str, &str, &str)] = &[
    ("24-hour clock", "%H:%M", "14:05"),
    ("12-hour clock", "%-I:%M %p", "2:05 PM"),
    ("Clock with day", "%a %H:%M", "Tue 14:05"),
];

/// One row per choice, the current one marked. A picks it and the screen
/// stays, so the change is seen on the rows themselves.
fn appearance() -> Items {
    let theme = pt35_common::load_theme();
    let dark = theme.apps.color_scheme != pt35_common::theme::ColorScheme::Light;
    let background = theme.color.background.to_string();
    let mut out: Items = PALETTES
        .iter()
        .map(|(name, colours)| {
            let get = |key: &str| colours.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
            Item {
                label: format!("{name} palette"),
                payload: format!("theme:palette={name}"),
                active: get("background") == Some(background.as_str()),
                // Five of these are near-black: the accent is what tells them apart.
                tint: get("accent").and_then(|hex| hex.parse().ok()),
                note: String::new(),
                ..Item::default()
            }
        })
        .collect();
    out.extend(ACCENTS.iter().map(|(name, hex)| {
        let tint: Option<Rgb> = hex.parse().ok();
        Item {
            label: format!("{name} accent"),
            payload: format!("theme:color.accent={hex}"),
            active: tint == Some(theme.color.accent),
            tint,
            ..Item::default()
        }
    }));
    out.extend(CLOCKS.iter().map(|(label, format, example)| Item {
        label: (*label).into(),
        payload: format!("theme:bar.clock_format={format}"),
        note: (*example).into(),
        icon: "preferences-system-time".into(),
        active: theme.bar.clock_format == *format,
        ..Item::default()
    }));
    out.push(Item {
        label: "Dark apps".into(),
        payload: format!(
            "theme:apps.color_scheme={}",
            if dark { "light" } else { "dark" }
        ),
        note: if dark { "on" } else { "off" }.into(),
        icon: "weather-clear-night".into(),
        ..Item::default()
    });
    let previews = theme.menu.window_previews;
    out.push(Item {
        label: "Window previews".into(),
        payload: format!("theme:menu.window_previews={}", !previews),
        note: if previews { "on" } else { "off" }.into(),
        icon: "preferences-system-windows".into(),
        ..Item::default()
    });
    let skull = theme.bar.menu_icon != "bars";
    out.push(Item {
        label: "Skull menu button".into(),
        payload: format!(
            "theme:bar.menu_icon={}",
            if skull { "bars" } else { "skull" }
        ),
        note: if skull { "on" } else { "off" }.into(),
        icon: "view-app-grid".into(),
        ..Item::default()
    });
    let toggles = [
        (
            "Row numbers",
            "menu.show_numbers",
            theme.menu.show_numbers,
            "view-list-ordered",
        ),
        (
            "Mode in bar",
            "bar.show_mode",
            theme.bar.show_mode,
            "input-keyboard",
        ),
        (
            "Animations",
            "menu.animations",
            theme.menu.animations,
            "desktop-effects",
        ),
        (
            "All .desktop apps",
            "menu.show_desktop_entries",
            theme.menu.show_desktop_entries,
            "applications-all",
        ),
    ];
    out.extend(toggles.into_iter().map(|(label, key, on, icon)| Item {
        label: label.into(),
        payload: format!("theme:{key}={}", !on),
        note: if on { "on" } else { "off" }.into(),
        icon: icon.into(),
        ..Item::default()
    }));
    out
}

/// `section.key=value` from an Appearance row, typed the way the theme wants it.
pub fn parse_pick(pick: &str) -> Option<(&str, &str, toml::Value)> {
    let (path, value) = pick.split_once('=')?;
    let (section, key) = path.split_once('.')?;
    let value = match value {
        "true" => toml::Value::Boolean(true),
        "false" => toml::Value::Boolean(false),
        _ => match value.parse::<i64>() {
            Ok(n) => toml::Value::Integer(n),
            Err(_) => toml::Value::String(value.into()),
        },
    };
    Some((section, key, value))
}

/// English, because the parsers read it: "Powered: yes", "enabled". UTF-8,
/// because plain C makes nmcli turn "Café" into something you cannot join.
/// Bounded, so a radio daemon that is not running costs a screen that says
/// nothing rather than one stuck on "Reading..." for good.
fn run(bin: &str, args: &[&str]) -> Option<String> {
    pt35_common::proc::output_within(
        Command::new(bin)
            .args(args)
            .env("LC_ALL", "C.UTF-8")
            .env_remove("LANGUAGE"),
        std::time::Duration::from_secs(10),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_thing_you_opened_is_at_the_top() {
        let mut items: Items = ["app:browser", "app:editor", "app:terminal", "exec:htop"]
            .iter()
            .map(|p| Item::new(*p, *p))
            .collect();
        promote_recent(
            &mut items,
            &["app:terminal".into(), "exec:htop".into(), "app:gone".into()],
        );
        let order: Vec<&str> = items.iter().map(|i| i.payload.as_str()).collect();
        assert_eq!(
            order,
            ["app:terminal", "exec:htop", "app:browser", "app:editor"]
        );
    }

    #[test]
    fn nothing_recent_leaves_the_order_alone() {
        let mut items: Items = ["a", "b"].iter().map(|p| Item::new(*p, *p)).collect();
        promote_recent(&mut items, &[]);
        assert_eq!(items[0].payload, "a");
    }
}

#[cfg(test)]
mod provider_tests {
    use super::*;

    #[test]
    fn an_app_without_a_profile_still_gets_a_name_and_two_letters() {
        assert_eq!(pretty_app("pt35-monitor"), "Monitor");
        assert_eq!(pretty_app("chromium"), "Chromium");
        assert_eq!(pretty_app(""), "Window");
        assert_eq!(initials("pcmanfm"), "PC");
        assert_eq!(initials("pt35-monitor"), "MO");
    }

    #[test]
    fn the_wifi_name_is_the_ssid_you_are_on() {
        let out = "no:Neighbours\nyes:Cafe\\: 5G\nno:\n";
        assert_eq!(active_wifi(out).as_deref(), Some("Cafe: 5G"));
        assert_eq!(active_wifi("no:Home\n"), None);
    }

    #[test]
    fn nmcli_escapes_are_undone_and_names_are_not_trimmed() {
        assert_eq!(nmcli_fields(r"yes:70:a\:b\\c"), ["yes", "70", r"a:b\c"]);
        let items = parse_nmcli("no:60: Lobby \nno:50:Cafe\\: 5G\n");
        assert_eq!(items[0].payload, " Lobby ");
        assert_eq!(items[1].payload, "Cafe: 5G");
    }

    #[test]
    fn a_terminal_is_known_by_the_terminal_and_by_what_it_runs() {
        assert_eq!(programs_of("foot pt35-hello"), ["foot", "pt35-hello"]);
        assert_eq!(
            program_of("foot -a pt35-monitor htop").as_deref(),
            Some("htop")
        );
        assert_eq!(
            program_of("/usr/bin/vlc --started-from-file").as_deref(),
            Some("vlc")
        );
    }

    #[test]
    fn field_codes_go_and_a_double_percent_stays() {
        assert_eq!(strip_field_codes("app %U"), "app");
        assert_eq!(strip_field_codes("app --file=%f --x"), "app --x");
        assert_eq!(strip_field_codes("printf 100%%"), "printf 100%");
    }

    #[test]
    fn desktop_values_are_unescaped_and_keys_may_have_spaces() {
        let text = "[Desktop Entry]\nType = Application\nName = Big\\sApp\nExec = big\n";
        let item = parse_desktop_entry(text).expect("an application");
        assert_eq!(item.label, "Big App");
        assert_eq!(item.payload, "big");
    }

    #[test]
    fn entries_for_other_desktops_or_missing_programs_are_hidden() {
        let entry =
            |extra: &str| format!("[Desktop Entry]\nType=Application\nName=x\nExec=x\n{extra}");
        let yes = |_: &str| true;
        let no = |_: &str| false;
        assert!(parse_entry(&entry("OnlyShowIn=GNOME;"), &["sway"], &yes).is_none());
        assert!(parse_entry(&entry("OnlyShowIn=GNOME;sway;"), &["sway"], &yes).is_some());
        assert!(parse_entry(&entry("NotShowIn=sway;"), &["sway"], &yes).is_none());
        assert!(parse_entry(&entry("TryExec=x"), &["sway"], &no).is_none());
        assert!(parse_entry(&entry("TryExec=x"), &["sway"], &yes).is_some());
    }

    #[test]
    fn reads_the_address_off_ip() {
        let out = "3: eth0    inet 192.168.1.20/24 brd 192.168.1.255 scope global eth0\\";
        assert_eq!(parse_address(out).as_deref(), Some("192.168.1.20"));
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn deduplicates_and_sorts_wifi_networks() {
        let out = "no:42:Cafe\nyes:78:Home\nno:55:Home\nno:0:\n";
        let items = parse_nmcli(out);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].label, "Home");
        assert!(items[0].active, "the active network is marked");
        assert_eq!(
            items[0].note, "connected  78%",
            "keeps the strongest signal"
        );
    }

    #[test]
    fn the_network_you_are_on_comes_first_even_when_weaker() {
        let items = parse_nmcli("no:90:Cafe\nyes:40:Home\n");
        assert_eq!(items[0].label, "Home");
        assert_eq!(items[1].note, "90%");
    }

    #[test]
    fn uptime_reads_in_the_two_units_that_matter() {
        assert_eq!(duration(59), "0m");
        assert_eq!(duration(4 * 3600 + 12 * 60 + 5), "4h 12m");
        assert_eq!(duration(3 * 86400 + 4 * 3600 + 59), "3d 4h");
    }

    #[test]
    fn the_os_name_loses_its_quotes() {
        let release = "NAME=\"Debian\"\nPRETTY_NAME=\"Debian GNU/Linux 13 (trixie)\"\n";
        assert_eq!(os_name(release).unwrap(), "Debian 13 (trixie)");
    }

    #[test]
    fn every_palette_sets_every_colour_and_its_text_reads() {
        for (name, colours) in PALETTES {
            assert_eq!(colours.len(), 15, "{name}");
            let get = |key: &str| -> Rgb {
                colours
                    .iter()
                    .find(|(k, _)| *k == key)
                    .unwrap()
                    .1
                    .parse()
                    .unwrap()
            };
            // Body text on the background, and text on the accent pill.
            for (fg, bg) in [
                ("foreground", "background"),
                ("accent_fg", "accent"),
                ("muted", "background"),
            ] {
                let (a, b) = (get(fg).luminance(), get(bg).luminance());
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(ratio >= 4.5, "{name}: {fg} on {bg} is {ratio:.1}:1");
            }
        }
    }

    #[test]
    fn an_accent_pick_brings_readable_text_with_it() {
        let (_, entries, replace) = crate::exec::picks("color.accent=#1f3a80").unwrap();
        assert!(!replace);
        assert!(entries.contains(&("accent_fg".into(), toml::Value::String("#f7f7f2".into()))));
        let (section, entries, replace) = crate::exec::picks("palette=Nord").unwrap();
        assert_eq!((section, entries.len(), replace), ("color", 15, true));
    }

    #[test]
    fn an_appearance_pick_is_typed() {
        let (section, key, value) = parse_pick("menu.show_numbers=false").unwrap();
        assert_eq!((section, key), ("menu", "show_numbers"));
        assert_eq!(value, toml::Value::Boolean(false));
        let (_, _, value) = parse_pick("bar.clock_format=%-I:%M %p").unwrap();
        assert_eq!(value, toml::Value::String("%-I:%M %p".into()));
        assert!(parse_pick("nonsense").is_none());
    }

    #[test]
    fn every_accent_parses_and_every_pick_is_a_real_theme_key() {
        for (_, hex) in ACCENTS {
            assert!(hex.parse::<Rgb>().is_ok(), "{hex}");
        }
        for item in appearance() {
            let pick = item.payload.strip_prefix("theme:").unwrap();
            let (section, entries, _) = crate::exec::picks(pick).unwrap();
            let mut table = toml::Table::new();
            let inner: toml::Table = entries.into_iter().collect();
            table.insert(section.into(), toml::Value::Table(inner));
            toml::Value::Table(table)
                .try_into::<pt35_common::theme::Theme>()
                .unwrap_or_else(|e| panic!("{pick}: {e}"));
        }
    }

    #[test]
    fn an_address_is_not_a_name() {
        assert!(is_unnamed("E1-78-69-05-2E-41"));
        assert!(!is_unnamed("RZR"));
        assert!(!is_unnamed("Keyboard K380"));
    }

    #[test]
    fn parses_bluetooth_devices() {
        let out = "Device AA:BB:CC:DD:EE:FF Keyboard K380\nnoise\n";
        assert_eq!(
            parse_bluetoothctl(out),
            vec![Item::new("Keyboard K380", "AA:BB:CC:DD:EE:FF")]
        );
    }

    #[test]
    fn parses_a_desktop_entry_and_strips_field_codes() {
        let text = "[Desktop Entry]\nType=Application\nName=Image Viewer\nExec=imv %U\nIcon=imv\n";
        let item = parse_desktop_entry(text).expect("an application");
        assert_eq!(item.label, "Image Viewer");
        assert_eq!(item.payload, "imv");
        assert_eq!(item.icon, "imv");
        assert_eq!(item.glyph, "I", "a letter when the theme has no such icon");
    }

    #[test]
    fn a_terminal_program_gets_a_terminal() {
        let text = "[Desktop Entry]\nType=Application\nName=Htop\nExec=htop\nTerminal=true\n";
        assert_eq!(
            parse_desktop_entry(text).unwrap().payload,
            "foot -a 'pt35-htop' htop"
        );
    }

    #[test]
    fn an_icon_path_is_not_a_theme_name() {
        let text = "[Desktop Entry]\nType=Application\nName=Thing\nExec=thing\nIcon=/usr/share/pixmaps/thing.png\n";
        assert_eq!(parse_desktop_entry(text).expect("an application").icon, "");
    }

    #[test]
    fn hides_nodisplay_and_non_application_entries() {
        assert_eq!(
            parse_desktop_entry(
                "[Desktop Entry]\nType=Application\nName=x\nExec=x\nNoDisplay=true\n"
            ),
            None
        );
        assert_eq!(
            parse_desktop_entry("[Desktop Entry]\nType=Link\nName=x\nExec=x\n"),
            None
        );
        assert_eq!(
            parse_desktop_entry("[Desktop Action new]\nName=x\nExec=x\n"),
            None,
            "only the main section counts"
        );
    }

    #[test]
    fn level_ladders_run_from_full_to_zero() {
        let items = levels("volume");
        assert_eq!(items.len(), 11);
        assert_eq!(items[0].payload, "volume 100");
        assert_eq!(items[10].payload, "volume 0");
    }
}
