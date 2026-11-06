# omarchy-super-productivity

An Omarchy shell bar widget (`Widget.qml`, `manifest.json`) that shows and controls the current Super Productivity task through the app's Local REST API, plus `launcher/` (`sp-launch`), a Rust tool that runs the Super Productivity AppImage unpacked in its own systemd user scope.

## Commands

- Launcher check, as CI runs it from `launcher/`: `cargo fmt --check && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked && cargo doc --no-deps --locked`
- Launcher install: `cd launcher && cargo build --release --locked && install -m 755 target/release/sp-launch ~/.local/bin/`
- Widget check (CI does not cover the widget): `omarchy plugin validate "$PWD"`
- Where the widget is installed decides how edits reach the bar. A normal `omarchy plugin add` install is its own clone in `~/.config/omarchy/plugins/saintchris.super-productivity`; after a merge, update it with `omarchy plugin update saintchris.super-productivity`. A development install where that folder is a symlink to this checkout is live on save (a save there reloads the bar); don't run `omarchy plugin update` against it, because it fast-forwards this checkout.

## Rules agents get wrong

- This repo is public. Keep personal names, work data, tokens and machine-specific paths out of code, docs, tests and commit messages.
- The widget talks only to `127.0.0.1` (the README promises nothing leaves the machine). Keep every request local.
- Each widget setting lives in three places: `manifest.json` (`defaults` and `schema`), its `setting(...)` default in `Widget.qml`, and the README Settings table. Change all three together.
- The launcher pins Rust 1.99.1 and forbids `unsafe`. Clippy warns on `unwrap` and `expect`, and CI turns warnings into errors, so outside tests return errors with `anyhow` and `?`.
- The AppImage fix needs both halves: the unpacked copy and the new systemd scope. Tested live, an unpacked copy left in the caller's scope still crashed when that scope ended. Keep both in `start`.
- `install` keeps the previous version for rollback and never deletes a version a running process uses. Keep both guarantees.
- Run a file that was just written through `retry_if_busy`: exec can fail with "Text file busy" (it failed once in CI).
- Strip `SCRUBBED_ENV` from the app's environment. Shells started by T3 Code set `ELECTRON_RUN_AS_NODE`, which makes Electron run as plain Node.
- Start Super Productivity with `sp-launch start -- <app arguments>`, also when restarting it from an agent's shell. Started from the AppImage or `AppRun` directly, the app dies when that shell's scope ends.
- To try `sp-launch` by hand, point `SP_LAUNCH_ROOT` at a temp folder so the installed versions in `~/.local/share/sp-launch` stay untouched. Tests use temp folders and fake `AppRun` scripts; keep them off the real store and the running app.
- `scripts/sp-update` installs a fetched build only while the desktop is idle or in the first minutes after login, and only after that same run has backed up the profile and read the current build's API (the current build is started on the real profile when the app is not already running). It installs only when every project and task id in the newest JSON backup is in that API. A new start counts only when the profile lock's executable is inside the unpacked folder `current` names. Rollback puts back that backup and the previous unpacked version, and reports the data restored only when the restarted app's API matches the snapshot. A staged AppImage leaves `current` alone. Keep `scripts/test-sp-update` on temp directories and a fake `sp-launch`.
