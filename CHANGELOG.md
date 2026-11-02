# Changelog

## Unreleased

### Fixed

- An app without a workspace of its own no longer takes another app's.
  Disk usage, DarkWire, a second terminal and any .desktop app now open on
  workspace 10 and up. Such an app used to open on the workspace in front.
  Opened over the Editor, it pushed the Editor off workspace 3. Opened on
  Music's empty workspace 5, it stayed there until Music opened, and then
  pt35d moved it onto another app's workspace (#6).
- A tenth window no longer splits the screen. pt35d looked for a free
  workspace only among 1 to 9, so the tenth window shared one with another
  app (#6).
- When pt35d moves an app to another workspace, its dialogs go with it.
  They used to stay behind, and the app waited on a dialog you could not see.
- Super + Enter opens the terminal in front. sway used to start foot itself,
  and the terminal's assign rule put it on workspace 1, out of sight unless
  you were on 1 already.

### Changed

- The bar and the menu use a tenth of the memory, and the menu opens in a
  quarter of the time. On a Pi 5 the bar takes 4.4 MB instead of 50 MB, the
  menu 6.1 MB instead of 61 MB, and the menu draws its first frame in 27 ms
  instead of 101 ms. The old font library read every glyph in all four
  DejaVu faces at each start. `ab_glyph` reads a glyph when it is first
  drawn. Text looks the same.
- When the terminal on workspace 1 closes, pt35d moves the oldest remaining
  terminal to 1, so Super + 1 still finds one.
- Settings is grouped. Appearance holds the palette, the accent, dark apps
  and animations. The new Bar screen holds the clock, the mode readout and
  the skull button. The new Menu screen holds row numbers, window previews
  and .desktop apps. Palette, Accent, Clock and Animations each open a
  screen of their own, and their row reads out the value in use. Appearance
  was one list of 20 rows. A `[menu.settings]` of your own in
  `~/.config/pt35/menu.toml` needs `{ builtin = "bar" }` and
  `{ builtin = "menu" }` rows to reach the settings that moved.
- Animations has a speed: Off, then 1 (slowest) to 5 (fastest). 3 is the
  old timing. `animations = true` and `false` in a theme file still work,
  as 3 and Off.

## 0.4.3 (2026-11-02)

### Added

- Settings > Appearance has an Animations switch. It stops the menu's motion
  in the menu that is open, without a restart. GTK apps follow it too, unless
  `[apps] color_scheme` is `keep`. Qt apps do not, because Qt's GTK theme
  never reads that setting (#4).

### Changed

- The D-pad pills in the hint bar show arrows instead of `<>` and `^v`. On a
  keyboard with `<` and `>` keys, `<>` looked like those two keys (#5).

### Fixed

- An app opened from the launcher now opens in front. If the app had a
  workspace of its own, focus went back to the window you came from while
  the app started, and the app opened out of sight (#2).
- L closes a window that has its menu bar open. A GTK app ignored the close
  while its menu was open, and the window came back after 3 seconds. pt35d
  now closes the menu first (#3).
- A `pt35ctl reload` that fails keeps the config that was running. pt35d used
  to keep the menu and apps files it had just rejected, half applied (#7).
- The installer works on a system in another language. It looked for the
  English "Candidate:" line in `apt-cache policy`. On a German system apt
  translates that line, so the installer found no sway and stopped with
  "no 'sway' package available". It now runs apt in English (#1).

## 0.4.2 (2026-10-21)

After upgrading, reflash the keyboard with `sudo pt35-kbd flash`.

### Fixed

- The keyboard works after a boot again. The keyboard sends a terminal title
  each time its console opens. Since 0.4.0, pt35d opened the console and only
  then ran `stty` to turn the echo off. In that gap, Linux echoed the title
  back to the keyboard with its emoji mangled into bytes that are not UTF-8.
  The pt35 firmware failed to read them and stopped, and every key was dead
  until the next power cycle. pt35d now turns the echo off itself, right
  after it opens the port. The firmware reads its console as raw bytes, so a
  bad byte no longer stops it.
- The firmware keeps reading keys after an error, such as a busy USB link.
  Before, any error stopped it until the next power cycle.
- `pt35-kbd` now restarts the keyboard firmware when the keyboard does not
  answer it. Before, it left the keyboard stopped.

## 0.4.1 (2026-10-19)

### Fixed

- The installer no longer stops halfway when a config file the package ships
  is already on disk. That happens on a move from a source install to the
  package, and after editing `/etc/keyd/pocketterm35.conf`. dpkg asked which
  copy to keep, and with no terminal the install failed. The installer now
  keeps the file on disk and puts the package's copy beside it as
  `.dpkg-dist`.
- The installer no longer tells you to flash a keyboard that already runs the
  pt35 firmware. It asks the running pt35d.

## 0.4.0 (2026-10-19)

After upgrading, reflash the keyboard with `sudo pt35-kbd flash` and run
`pt35-ai agents --reset` if you use the AI agent. The firmware and the agent
rules below only take effect then.

### Security

- Joining a Wi-Fi network no longer runs its name as a shell command. Before,
  picking a nearby network named `$(...)` ran that command. The menu now passes
  the name to nmcli as an argument.
- The Default and Code agents ask before they run `git diff` or `git log`. The
  `--output` flag of either can overwrite any file.

### Fixed

- Pressing `2` on a confirmation such as Shut down or Close all no longer
  answers Yes.
- The face buttons keep working after a sway reload with `$mod+Shift+c`. pt35d
  sees the reload and binds them again.
- The patched firmware prints nothing but its brightness answer. Its debug
  output could fill the console buffer and stop the keyboard.
- `B50` now reads back 50, not 49. Fn `-` stops at 5% instead of turning the
  panel off. Letting go of Fn before Select takes the screenshot and no longer
  opens the switcher.
- A typo in one theme file now drops only that file, and the menu shows which
  file and line. The next Appearance pick no longer wipes a broken
  `appearance.toml`. The shell writes picks, pins and recents to a temporary
  file and renames it, so a power cut cannot leave half a file.
- When a window closes, the switcher's cursor stays on the window it was on.
  Closing a card behind you no longer moves you to another app.
- A search you are typing stays when a Bluetooth or Wi-Fi scan refreshes the
  list.
- A tap on a filtered Quick panel changes the row you tapped. The sliders move
  on each press instead of a press or two later.
- Each app with a profile shows once in the launcher and always starts with
  its profile. A `.desktop` file in `~/.local/share/applications` overrides the
  system copy, and one with `Hidden=true` hides the app. The launcher reads
  `XDG_DATA_DIRS` and checks `TryExec`, `OnlyShowIn` and `NotShowIn`.
- You can join a Wi-Fi network whose name has a `:` or `\` in it. The Quick
  panel shows the network's SSID and whether the radio is on.
- Apps started from the shell no longer stay behind as zombie processes.
- pt35d asks a stock keyboard for its firmware three times per plug-in instead
  of every 10 seconds, so `pt35-kbd flash` gets through. It notices when
  `pt35-kbd restore` takes the patched firmware away.
- With pt35d not running, the menu can still close. The sway config restarts
  pt35d and the bar when they die.
- A helper that hangs, such as wpctl, bluetoothctl, nmcli or grim, no longer
  hangs the daemon or a menu screen. Each one now has a time limit.
- The bar shows the wired icon only with a cable in, and never a `--` battery.
  With many windows open it shrinks the dock slots instead of drawing them
  under the status icons.
- The menu draws the last frame of each glide, so the highlight lands exactly
  on its row.
- Menu errors are visible. The hint bar used to cover the error line.
- The wheel stops at the end of a list instead of jumping to the other end.
  Releasing a scrollbar drag over a row no longer opens that row. In Mouse
  mode, tapping the B pill opens the row's options, as pressing B does.
- Uninstalling a source install through curl now also undoes the greetd setup.
- postinst no longer creates a missing `~/.config` owned by root.
- `pt35-ai` downloads a model again when its pinned checksum changes.
- `pt35-kbd write` checks the new file before it replaces the old one.

### Changed

- A, B and X no longer repeat in Buttons mode. Holding one while the menu
  opened could crash sway 1.10.
- Qt apps fall back to XWayland when the Qt Wayland plugin is missing.

## 0.3.2 (2026-09-24)

### Fixed

- An app's context menu no longer covers the launcher. Opening the menu over
  a right-click menu left the popup on top, holding the keyboard. The popup
  now closes when the menu opens.
- The DarkWire Default agent stops reaching for sudo. It tried
  `sudo journalctl`, the sudo rule refused it, and it gave up. The prompt now
  says logs need no sudo and shows the command for recent errors. Run
  `pt35-ai agents --reset` to pick up the new prompt.

## 0.3.1 (2026-09-24)

### Added

- A local AI agent: `pt35-ai` installs llama.cpp, llama-swap and pinned
  models, with darkwire as the chat client and five agents. The DarkWire
  tile opens it.

### Changed

- The menu is the desktop when nothing is open on the workspace in front,
  so an app that exits leaves you on the launcher.

### Fixed

- Lock was a white screen: swaylock with no config paints white. It now
  uses the theme's colours, unless you have a swaylock config of your own.
- Screen off had no way back but ssh. Any key or touch now turns the
  panel on again. This needs `swayidle`, now a dependency.
- The power key no longer opens the power menu under the lock, where it
  waited after the unlock with Screen off selected.

## 0.3.0 (2026-09-23)

First release.
