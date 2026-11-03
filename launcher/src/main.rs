//! `sp-launch`: install and start Super Productivity from an unpacked AppImage.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use sp_launch::{Store, launch, running_apps};

#[derive(Parser)]
#[command(
    version,
    about = "Run Super Productivity from an unpacked AppImage, so it never dies with whatever started it"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Unpack an AppImage and make it the version `start` runs. Keeps the previous version.
    Install { appimage: PathBuf },
    /// Start the current version with these arguments (pass them after `--`).
    Start {
        /// Run it in the caller's systemd scope instead of a new one.
        #[arg(long)]
        no_scope: bool,
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    /// Show the installed version and the running copies.
    Status,
}

fn main() -> Result<()> {
    let store = Store::from_env()?;
    match Cli::parse().command {
        Cmd::Install { appimage } => {
            let done = store.install(&appimage, Store::versions_in_use)?;
            let how = if done.unpacked {
                "unpacked"
            } else {
                "already unpacked"
            };
            println!(
                "{} {how}; current is now {}",
                done.version,
                store.versions_dir().join(&done.version).display()
            );
            if let Some(prev) = done.previous {
                println!("previous version {prev} kept for rollback");
            }
            for v in done.removed {
                println!("removed old version {v}");
            }
            println!("running copies keep their version until they restart");
        }
        Cmd::Start { no_scope, args } => {
            let log_path = store.root().join("start.log");
            let log = open_log(&log_path);
            let grace = Duration::from_secs(2);
            let mut pid = None;
            if !no_scope && which("systemd-run") {
                match launch(store.start_command(&args, true, log.as_ref())?, grace) {
                    Ok(p) => pid = Some(p),
                    // No user systemd (an ssh shell, a cron job): starting without a scope
                    // still beats not starting.
                    Err(e) => eprintln!(
                        "warning: {e:#} in a new systemd scope (see {}); starting without one",
                        log_path.display()
                    ),
                }
            }
            let pid = match pid {
                Some(p) => p,
                None => launch(store.start_command(&args, false, log.as_ref())?, grace)
                    .with_context(|| format!("see {}", log_path.display()))?,
            };
            println!("started (pid {pid})");
        }
        Cmd::Status => {
            match store.current() {
                Some(v) => {
                    println!("current: {v} ({})", store.versions_dir().join(&v).display());
                    if let Some(src) = store.source_of(&v) {
                        println!("from:    {src}");
                    }
                }
                None => println!("current: none (run `sp-launch install <AppImage>`)"),
            }
            let apps = running_apps();
            if apps.is_empty() {
                println!("running: none");
            }
            for (pid, exe, fuse) in apps {
                let note = if fuse {
                    "  <- from an AppImage mount: dies with whatever started it"
                } else {
                    ""
                };
                println!("running: pid {pid} {}{note}", exe.display());
            }
        }
    }
    Ok(())
}

/// The app's output log, emptied once it passes 1 MB. None when it can't be opened.
fn open_log(path: &Path) -> Option<File> {
    if fs::metadata(path).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = fs::remove_file(path);
    }
    fs::create_dir_all(path.parent()?).ok()?;
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}
