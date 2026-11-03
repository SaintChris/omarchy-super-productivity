# Super Productivity for the Omarchy bar

A bar widget for the [Omarchy](https://omarchy.org/) shell that shows the task you're tracking in [Super Productivity](https://super-productivity.com/) and how long you've spent on it today. You can start and stop tasks from the bar, so the app can stay hidden in the tray.

![The widget showing a running task](docs/bar.png)

## What it does

| Action | Result |
| --- | --- |
| Left click while a task runs | Stops tracking |
| Left click while nothing runs | Opens today's tasks; click one to start it |
| Right click | Opens today's tasks |
| Middle click | Shows or hides the Super Productivity window |
| Click while the app isn't running | Starts the app |

The widget only talks to the app's Local REST API on `127.0.0.1`. Nothing leaves your machine.

Today's time follows the calendar date. If you moved the start of your day past midnight in Super Productivity (Settings > General > Misc Settings), the bar shows 0:00 from midnight until then.

## Install

```bash
omarchy plugin add https://github.com/SaintChris/omarchy-super-productivity.git --enable --yes
```

The widget lands on the right of the bar. Move it with `omarchy bar move saintchris.super-productivity --section center`.

## Set up Super Productivity

1. In Super Productivity, open **Settings > General > Misc Settings** and turn on **Enable local REST API**. This needs version 18.17 or later.
2. In the same place, turn on **Minimize to tray**. Closing the window then hides the app in the tray instead of quitting it, and the widget keeps working.
3. To start the app at login, add it to `~/.config/hypr/autostart.lua`:

   ```lua
   o.launch_on_start("superproductivity")
   ```

   For an AppImage, start it through `sp-launch` instead (see below).

## Settings

The defaults suit a standard install. Change them with `omarchy bar set`:

| Key | Default | What it is |
| --- | --- | --- |
| `port` | `3876` | REST API port |
| `tokenFile` | `~/.config/superProductivity/local-rest-api-token` | File holding the API token |
| `launchCommand` | `superproductivity` | Command that starts the app (AppImage path, Flatpak command and so on) |
| `maxWidth` | `220` | Longest task title in the bar, in pixels |

For example, for an AppImage with its own data folder, started through `sp-launch` (below):

```bash
omarchy bar set saintchris.super-productivity launchCommand "$HOME/.local/bin/sp-launch start -- --user-data-dir=$HOME/.config/sp-work"
omarchy bar set saintchris.super-productivity tokenFile "~/.config/sp-work/local-rest-api-token"
```

## Running the AppImage: `sp-launch`

Started straight from an AppImage, Super Productivity can die with SIGBUS long after it started. The AppImage's runtime serves the app's files from a FUSE mount and stays in the systemd scope of whatever started it (Hyprland's autostart, a terminal, an agent's shell). When that scope ends (Hyprland crashes, a reboot, the terminal closes) the mount disappears and the app is killed in the middle of whatever it was doing. Electron moves its main process into a scope of its own, but the helpers it starts first (zygote, Wayland idle helper) stay behind, so even an unpacked copy crashes (SIGTRAP) when the starter's scope is stopped.

`launcher/` is a small Rust tool that fixes both:

- `sp-launch install <AppImage>` unpacks the AppImage once into `~/.local/share/sp-launch/versions/<sha256 prefix>/` and points `current` at it. It keeps the previous version for rollback and never deletes one a running process uses.
- `sp-launch start -- <app arguments>` runs the current version's `AppRun` in a new systemd user scope, with `ELECTRON_RUN_AS_NODE` and the AppImage variables removed, so the app outlives whatever started it. If the app is already running, it hands the request to the running copy and quits, as usual.
- `sp-launch status` shows the installed version and each running copy, and flags one still running from an AppImage mount.

```bash
cd launcher && cargo build --release --locked && install -m 755 target/release/sp-launch ~/.local/bin/
sp-launch install ~/Applications/Super-Productivity.AppImage
```

Then use `sp-launch start -- <arguments>` in `autostart.lua`, in `launchCommand` and in any desktop entry. Run `sp-launch install` again after each new AppImage; a running copy keeps its version until it restarts. To roll back, point the link at the kept version: `ln -sfn versions/<previous> ~/.local/share/sp-launch/current`.

## License

MIT
