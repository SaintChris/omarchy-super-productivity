//! Run Super Productivity's AppImage from an unpacked copy.
//!
//! An AppImage serves the app's files from a FUSE mount that its own runtime
//! process provides. Electron moves itself into its own systemd scope, but the
//! runtime stays in the scope of whatever started it (Hyprland's autostart, a
//! terminal, a T3 Code chat). When that scope ends, the mount disappears under
//! the running app and the kernel kills it with SIGBUS. Running the app from
//! an unpacked copy removes the mount, so nothing else can take the app down.
//!
//! Layout under the store root (`~/.local/share/sp-launch`):
//! `versions/<first 16 hex of the AppImage's sha256>/` holds each unpacked
//! copy, and `current` is a relative symlink to the one in use.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::unix::fs::symlink;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// File inside each unpacked version recording where it came from.
pub const SOURCE_FILE: &str = ".sp-launch-source";

/// Environment the launcher must not pass on. `ELECTRON_RUN_AS_NODE` (set in
/// shells that T3 Code starts) makes Electron run as plain Node; the AppImage
/// variables would point `AppRun` at a mount that may no longer exist.
pub const SCRUBBED_ENV: [&str; 5] = ["ELECTRON_RUN_AS_NODE", "APPDIR", "APPIMAGE", "ARGV0", "OWD"];

/// The unpacked versions and the `current` link.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

/// What `install` did.
#[derive(Debug, PartialEq, Eq)]
pub struct Installed {
    pub version: String,
    /// False when that AppImage was already unpacked.
    pub unpacked: bool,
    pub previous: Option<String>,
    pub removed: Vec<String>,
}

impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `$SP_LAUNCH_ROOT`, else `$XDG_DATA_HOME/sp-launch`, else `~/.local/share/sp-launch`.
    ///
    /// # Errors
    /// Fails when none of those variables is set.
    pub fn from_env() -> Result<Self> {
        if let Some(root) = std::env::var_os("SP_LAUNCH_ROOT") {
            return Ok(Self::new(root));
        }
        let data = match std::env::var_os("XDG_DATA_HOME") {
            Some(d) if !d.is_empty() => PathBuf::from(d),
            _ => PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
                .join(".local/share"),
        };
        Ok(Self::new(data.join("sp-launch")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("versions")
    }

    fn current_link(&self) -> PathBuf {
        self.root.join("current")
    }

    /// The version `current` points to, if any.
    pub fn current(&self) -> Option<String> {
        let target = fs::read_link(self.current_link()).ok()?;
        target.file_name().map(|n| n.to_string_lossy().into_owned())
    }

    /// Path of a version's `AppRun`.
    pub fn app_run(&self, version: &str) -> PathBuf {
        self.versions_dir().join(version).join("AppRun")
    }

    /// The AppImage path and full sha256 recorded when a version was unpacked.
    pub fn source_of(&self, version: &str) -> Option<String> {
        let text = fs::read_to_string(self.versions_dir().join(version).join(SOURCE_FILE)).ok()?;
        Some(text.trim().to_owned())
    }

    /// Unpack an AppImage (unless that exact file was unpacked before), point
    /// `current` at it, and delete every other version except the previous one
    /// and any that a running process uses. `in_use` is asked after the unpack,
    /// right before pruning, so an app started meanwhile is counted.
    ///
    /// # Errors
    /// Fails when the AppImage can't be read or unpacked, or the store can't be written.
    pub fn install(
        &self,
        appimage: &Path,
        in_use: impl FnOnce(&Self) -> HashSet<String>,
    ) -> Result<Installed> {
        let appimage = fs::canonicalize(appimage)
            .with_context(|| format!("can't find {}", appimage.display()))?;
        let sha = sha256_file(&appimage)?;
        let version = sha[..16].to_owned();
        let dest = self.versions_dir().join(&version);
        fs::create_dir_all(self.versions_dir())
            .with_context(|| format!("can't create {}", self.versions_dir().display()))?;

        let unpacked = !dest.join("AppRun").is_file();
        if unpacked {
            // A damaged copy (AppRun gone) would block the rename below.
            if dest.exists() {
                fs::remove_dir_all(&dest).context("can't remove a damaged copy of this version")?;
            }
            self.unpack(&appimage, &dest)?;
            fs::write(
                dest.join(SOURCE_FILE),
                format!("{sha}  {}\n", appimage.display()),
            )
            .context("can't record the AppImage's source")?;
        }

        let previous = self
            .current()
            .filter(|p| *p != version && self.app_run(p).is_file());
        self.point_current_at(&version)?;

        let mut keep = in_use(self);
        keep.insert(version.clone());
        keep.extend(previous.clone());
        let removed = self.prune(&keep)?;
        Ok(Installed {
            version,
            unpacked,
            previous,
            removed,
        })
    }

    fn unpack(&self, appimage: &Path, dest: &Path) -> Result<()> {
        let staging = self
            .versions_dir()
            .join(format!(".staging-{}", std::process::id()));
        if staging.exists() {
            fs::remove_dir_all(&staging).context("can't clear an old staging folder")?;
        }
        fs::create_dir(&staging).context("can't create the staging folder")?;
        // The runtime unpacks into ./squashfs-root, so run it inside the staging folder.
        let status = Command::new(appimage)
            .arg("--appimage-extract")
            .current_dir(&staging)
            .env_remove("ELECTRON_RUN_AS_NODE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .with_context(|| format!("can't run {}", appimage.display()))?;
        let root = staging.join("squashfs-root");
        if !status.success() || !root.join("AppRun").is_file() {
            let _ = fs::remove_dir_all(&staging);
            bail!(
                "{} did not unpack ({status}); is it an AppImage?",
                appimage.display()
            );
        }
        let moved = fs::rename(&root, dest).context("can't move the unpacked copy into place");
        let _ = fs::remove_dir_all(&staging);
        moved
    }

    /// Swap `current` in one rename, so a launch never sees it missing.
    fn point_current_at(&self, version: &str) -> Result<()> {
        let tmp = self.root.join(format!(".current-{}", std::process::id()));
        let _ = fs::remove_file(&tmp);
        symlink(Path::new("versions").join(version), &tmp).context("can't create the new link")?;
        fs::rename(&tmp, self.current_link()).context("can't switch the current link")?;
        Ok(())
    }

    /// Delete every version not in `keep`, and staging folders left by installs
    /// that were interrupted. Returns the versions deleted.
    ///
    /// # Errors
    /// Fails when the versions folder can't be read or a version can't be deleted.
    pub fn prune(&self, keep: &HashSet<String>) -> Result<Vec<String>> {
        let mut removed = Vec::new();
        for entry in fs::read_dir(self.versions_dir()).context("can't read the versions folder")? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(pid) = name.strip_prefix(".staging-") {
                if !Path::new("/proc").join(pid).exists() {
                    let _ = fs::remove_dir_all(entry.path());
                }
                continue;
            }
            if name.starts_with('.') || keep.contains(&name) || !entry.file_type()?.is_dir() {
                continue;
            }
            fs::remove_dir_all(entry.path())
                .with_context(|| format!("can't delete version {name}"))?;
            removed.push(name);
        }
        removed.sort();
        Ok(removed)
    }

    /// Versions that running processes were started from (from `/proc/*/exe`).
    pub fn versions_in_use(&self) -> HashSet<String> {
        // /proc/*/exe holds canonical paths; the store root may contain a symlink.
        let versions =
            fs::canonicalize(self.versions_dir()).unwrap_or_else(|_| self.versions_dir());
        running_exes()
            .into_iter()
            .filter_map(|(_, exe)| {
                let rest = exe.strip_prefix(&versions).ok()?;
                rest.components()
                    .next()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
            })
            .collect()
    }

    /// The command that starts the current version with `args`, in its own
    /// systemd user scope when `scope` is true, so it outlives its caller's scope.
    /// The app's output goes to `log` (or nowhere).
    ///
    /// # Errors
    /// Fails when nothing is installed yet, or the log can't be shared with the child.
    pub fn start_command(
        &self,
        args: &[OsString],
        scope: bool,
        log: Option<&fs::File>,
    ) -> Result<Command> {
        let version = self
            .current()
            .context("nothing installed yet: run `sp-launch install <AppImage>` first")?;
        let app_run = self.app_run(&version);
        if !app_run.is_file() {
            bail!(
                "{} is missing: run `sp-launch install <AppImage>` again",
                app_run.display()
            );
        }
        let mut cmd = if scope {
            let mut c = Command::new("systemd-run");
            c.args([
                "--user",
                "--scope",
                "--collect",
                "--quiet",
                "--description=Super Productivity (sp-launch)",
            ]);
            c.arg(&app_run);
            c
        } else {
            Command::new(&app_run)
        };
        cmd.args(args);
        for var in SCRUBBED_ENV {
            cmd.env_remove(var);
        }
        // Own process group and no terminal: closing the caller's terminal sends it nothing.
        let (out, err) = match log {
            Some(f) => (Stdio::from(f.try_clone()?), Stdio::from(f.try_clone()?)),
            None => (Stdio::null(), Stdio::null()),
        };
        cmd.process_group(0)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err);
        Ok(cmd)
    }
}

/// Start the app and return its PID. Waits up to `grace` for an early failure:
/// a quick exit with status 0 is fine (Super Productivity hands the request to
/// an already running copy and quits).
///
/// # Errors
/// Fails when the command can't be started or exits non-zero within `grace`.
pub fn launch(mut cmd: Command, grace: Duration) -> Result<u32> {
    let mut child = cmd.spawn().context("can't start Super Productivity")?;
    let pid = child.id();
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(pid);
            }
            bail!("Super Productivity exited at once ({status})");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(pid)
}

/// Lowercase hex sha256 of a file.
///
/// # Errors
/// Fails when the file can't be read.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("can't open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `(pid, exe)` for every process whose executable this user can read.
pub fn running_exes() -> Vec<(u32, PathBuf)> {
    let Ok(dir) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.filter_map(|e| {
        let e = e.ok()?;
        let pid: u32 = e.file_name().to_str()?.parse().ok()?;
        let exe = fs::read_link(e.path().join("exe")).ok()?;
        Some((pid, exe))
    })
    .collect()
}

/// Super Productivity main processes (not Electron helpers): `(pid, exe, runs from an AppImage mount)`.
pub fn running_apps() -> Vec<(u32, PathBuf, bool)> {
    let mut apps: Vec<_> = running_exes()
        .into_iter()
        .filter(|(_, exe)| {
            exe.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("superproductivity"))
        })
        .filter(|(pid, _)| {
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            // Chromium rewrites its helpers' command lines as one space-joined string,
            // so look for the flag anywhere rather than per argument.
            !cmdline.windows(7).any(|w| w == b"--type=")
        })
        .map(|(pid, exe)| {
            // The runtime mounts at $TMPDIR/.mount_<name><random>.
            let fuse = exe.to_string_lossy().contains("/.mount_");
            (pid, exe, fuse)
        })
        .collect();
    apps.sort();
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in AppImage: `--appimage-extract` writes squashfs-root/AppRun,
    /// which records its arguments and environment when run. `tag` changes the hash.
    fn fake_appimage(dir: &Path, name: &str, tag: &str) -> PathBuf {
        let path = dir.join(name);
        let script = format!(
            "#!/bin/sh\n# {tag}\n[ \"$1\" = --appimage-extract ] || exit 3\nmkdir -p squashfs-root\n\
             printf '#!/bin/sh\\necho \"$*\" > \"$SP_TEST_OUT\"\\necho \"node=${{ELECTRON_RUN_AS_NODE:-unset}}\" >> \"$SP_TEST_OUT\"\\n' > squashfs-root/AppRun\n\
             chmod +x squashfs-root/AppRun\n"
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn install_unpacks_and_points_current_at_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let img = fake_appimage(tmp.path(), "a.AppImage", "one");
        let done = store.install(&img, |_| HashSet::new()).unwrap();
        assert!(done.unpacked);
        assert_eq!(done.previous, None);
        assert_eq!(store.current().as_deref(), Some(done.version.as_str()));
        assert!(store.app_run(&done.version).is_file());
        assert_eq!(
            fs::read_link(store.root().join("current")).unwrap(),
            Path::new("versions").join(&done.version)
        );
        assert!(
            store
                .source_of(&done.version)
                .unwrap()
                .ends_with("a.AppImage")
        );
        // No staging folder left behind.
        let left: Vec<_> = fs::read_dir(store.versions_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn reinstalling_the_same_file_does_not_unpack_again() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let img = fake_appimage(tmp.path(), "a.AppImage", "one");
        let first = store.install(&img, |_| HashSet::new()).unwrap();
        let again = store.install(&img, |_| HashSet::new()).unwrap();
        assert!(!again.unpacked);
        assert_eq!(again.version, first.version);
        assert_eq!(again.previous, None);
    }

    #[test]
    fn keeps_the_previous_version_and_prunes_older_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let v1 = store
            .install(&fake_appimage(tmp.path(), "1", "one"), |_| HashSet::new())
            .unwrap()
            .version;
        let v2 = store
            .install(&fake_appimage(tmp.path(), "2", "two"), |_| HashSet::new())
            .unwrap()
            .version;
        let third = store
            .install(&fake_appimage(tmp.path(), "3", "three"), |_| HashSet::new())
            .unwrap();
        assert_eq!(third.previous.as_deref(), Some(v2.as_str()));
        assert_eq!(third.removed, vec![v1.clone()]);
        assert!(!store.versions_dir().join(&v1).exists());
        assert!(store.versions_dir().join(&v2).exists());
    }

    #[test]
    fn never_deletes_a_version_a_process_still_runs_from() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let v1 = store
            .install(&fake_appimage(tmp.path(), "1", "one"), |_| HashSet::new())
            .unwrap()
            .version;
        store
            .install(&fake_appimage(tmp.path(), "2", "two"), |_| HashSet::new())
            .unwrap();
        let in_use = HashSet::from([v1.clone()]);
        let third = store
            .install(&fake_appimage(tmp.path(), "3", "three"), |_| in_use)
            .unwrap();
        assert!(third.removed.is_empty());
        assert!(store.versions_dir().join(&v1).exists());
    }

    #[test]
    fn a_file_that_is_not_an_appimage_is_refused_and_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let bad = tmp.path().join("bad");
        fs::write(&bad, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();
        let err = store
            .install(&bad, |_| HashSet::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not unpack"), "{err}");
        assert_eq!(store.current(), None);
        assert_eq!(fs::read_dir(store.versions_dir()).unwrap().count(), 0);
    }

    #[test]
    fn start_runs_app_run_with_the_args_and_a_scrubbed_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        store
            .install(&fake_appimage(tmp.path(), "a", "one"), |_| HashSet::new())
            .unwrap();
        let out = tmp.path().join("out");
        let mut cmd = store
            .start_command(
                &["--rest-api-port=3877".into(), "--start-minimized".into()],
                false,
                None,
            )
            .unwrap();
        // Every scrubbed variable is removed from the child's environment.
        for var in SCRUBBED_ENV {
            assert!(
                cmd.get_envs().any(|(k, v)| k == var && v.is_none()),
                "{var} not removed"
            );
        }
        cmd.env("SP_TEST_OUT", &out);
        launch(cmd, Duration::from_secs(5)).unwrap();
        // launch() returns once the child is running; wait for the stand-in to finish writing.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !fs::read_to_string(&out).is_ok_and(|t| t.contains("node="))
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        let text = fs::read_to_string(&out).unwrap();
        assert!(
            text.starts_with("--rest-api-port=3877 --start-minimized\n"),
            "{text}"
        );
    }

    #[test]
    fn a_damaged_copy_is_unpacked_again() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let img = fake_appimage(tmp.path(), "a", "one");
        let v = store.install(&img, |_| HashSet::new()).unwrap().version;
        fs::remove_file(store.app_run(&v)).unwrap();
        let again = store.install(&img, |_| HashSet::new()).unwrap();
        assert!(again.unpacked);
        assert!(store.app_run(&v).is_file());
    }

    #[test]
    fn a_missing_previous_version_is_not_reported_as_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let v1 = store
            .install(&fake_appimage(tmp.path(), "1", "one"), |_| HashSet::new())
            .unwrap()
            .version;
        fs::remove_dir_all(store.versions_dir().join(&v1)).unwrap();
        let second = store
            .install(&fake_appimage(tmp.path(), "2", "two"), |_| HashSet::new())
            .unwrap();
        assert_eq!(second.previous, None);
    }

    #[test]
    fn staging_left_by_an_interrupted_install_is_cleaned_up() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path().join("store"));
        let stale = store.versions_dir().join(".staging-999999999");
        fs::create_dir_all(stale.join("squashfs-root")).unwrap();
        store
            .install(&fake_appimage(tmp.path(), "a", "one"), |_| HashSet::new())
            .unwrap();
        assert!(!stale.exists());
    }

    #[test]
    fn in_use_is_found_through_a_symlinked_store_root() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        symlink(&real, tmp.path().join("link")).unwrap();
        let store = Store::new(tmp.path().join("link"));
        let v = store
            .install(&fake_appimage(tmp.path(), "a", "one"), |_| HashSet::new())
            .unwrap()
            .version;
        // Run a copy of `sleep` from inside the version folder, so /proc/<pid>/exe
        // shows the canonical path under real/.
        let sleep = store.versions_dir().join(&v).join("sleep");
        fs::copy("/bin/sleep", &sleep).unwrap();
        let mut child = Command::new(&sleep).arg("5").spawn().unwrap();
        let found = store.versions_in_use();
        child.kill().unwrap();
        let _ = child.wait();
        assert!(found.contains(&v), "{found:?}");
    }

    #[test]
    fn start_before_install_explains_what_to_do() {
        let tmp = tempfile::tempdir().unwrap();
        let err = Store::new(tmp.path())
            .start_command(&[], false, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains("sp-launch install"), "{err}");
    }

    #[test]
    fn sha256_matches_a_known_value() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("f");
        fs::write(&f, "abc").unwrap();
        assert_eq!(
            sha256_file(&f).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
