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

### Getting an AppImage built elsewhere: `sp-fetch`

Building Super Productivity takes several GB of memory and most of an hour, so a laptop can leave it to another machine and fetch the result. `scripts/sp-fetch` reads a folder on that machine over SSH, checks the AppImage against its sha256 and installs it with `sp-launch install`:

```bash
install -m 755 scripts/sp-fetch ~/.local/bin/
sp-fetch <ssh host> <folder> --check       # what the build machine has
sp-fetch <ssh host> <folder>               # fetch, check, sp-launch install
sp-fetch <ssh host> <folder> --no-install  # fetch and check only
```

The folder holds the AppImage (`superProductivity-arm64-<commit>.AppImage`), `latest.sha256` (one `sha256sum` line naming it) and `latest.txt` (branch, commit, build time). The AppImage lands in `~/Applications` (or `SP_FETCH_DEST`), and the two newest are kept. A running copy keeps its version until it restarts.

### Following a build automatically: `sp-update`

`scripts/sp-update` is the unattended form of `sp-fetch`. A systemd user timer runs it at 08:00, 14:00 and 20:00 local time, and again two minutes after login. A newer AppImage is downloaded and checked against its sha256 as soon as the timer runs. That download does not change which version launches. `current` stays put until a later run both installs and starts the new build.

That run waits until the desktop has been idle, or until the session is still in its first minutes after login. It then does the switch in one go: read the current build's local API, quit Super Productivity, back up the profile, `sp-launch install`, start the app, and check that the API on `127.0.0.1` is up and the same projects and tasks are present. The backup is taken after the app has quit, because that is the copy a rollback restores. An overnight reboot keeps running the version already installed.

The before-state is that API snapshot. Electron rewrites the main process command line into one string and drops `--user-data-dir`, so a running profile is the pid in its `SingletonLock` symlink (`<host>-<pid>`) when that process's executable starts with `superproductivity`. When the app is not already running, the current build is started against a throwaway copy of the profile on another port, the API is read there, and the copy is discarded. The newest `backups/*.json` is not the before-state. If it lists a project or task the API does not have, nothing is installed and a running app is left up.

The backup is the profile directory without the browser caches, plus that newest JSON file. Five dated copies are kept. If the backup fails, or the current app does not answer the API, nothing is installed. If the new version does not come up, or projects or tasks are missing, `sp-update` points `current` back at the previous version, restores that backup and starts the previous version. It reports that the data backup was restored only when that restarted app's API matches the snapshot taken before the install. Sync settings are copied with the profile and are not edited.

Each install, skipped install or failure sends one ntfy message. The topic is the contents of the file named by `SP_UPDATE_NTFY_TOPIC_FILE`. It is not stored in this repo. Set `SP_UPDATE_NTFY_SINK` to a path to record the message locally instead of sending it. A missed run while the machine was asleep happens the next time the timer starts. The timer does not hold up login. The first start can catch up a calendar slot that already passed; that run still installs only when the desktop is idle or the session just started.

```bash
install -m 755 scripts/sp-update ~/.local/bin/
install -m 644 systemd/sp-update.service systemd/sp-update.timer ~/.config/systemd/user/
```

Settings are a `KEY=value` file, `~/.config/sp-update/config` (mode `0600`), or the same names in the environment. A variable that is already set wins over the file.

```
SP_UPDATE_HOST=example
SP_UPDATE_DIR=/srv/super-productivity
SP_UPDATE_DATA_DIR=$HOME/.config/sp-work
SP_UPDATE_NTFY_TOPIC_FILE=$HOME/.config/sp-update/ntfy-topic
SP_UPDATE_START_ARGS=--user-data-dir=$HOME/.config/sp-work --rest-api-port=3876
```

`SP_UPDATE_START_ARGS` is used when the app is not running, and when the running process's arguments cannot be read. It has to name the same profile as `SP_UPDATE_DATA_DIR`. When those arguments are readable, they are used instead. `SP_UPDATE_PROBE_PORT`, when set, is the port for the throwaway check; otherwise the script picks a free port other than the app's.

```bash
systemctl --user daemon-reload
systemctl --user enable --now sp-update.timer
sp-update --check
```

`sp-update --check` only compares the delivered build with the installed one.

## License

MIT
