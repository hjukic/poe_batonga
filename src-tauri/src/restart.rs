//! Restart with the newest version: pull the repository the app was built from,
//! rebuild it, and run the new build in place of this one. The running overlay
//! stays usable while it builds; a failed step leaves it running and says why.
//!
//! Under `npm run desktop` (a debug build) `tauri dev` already rebuilds and restarts
//! on every change, so the button only pulls and then nudges it to restart.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager};

static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, serde::Serialize)]
pub struct RestartStatus {
    /// What is happening now; empty when idle.
    pub step: String,
    pub error: Option<String>,
}

/// The repository root (this crate lives in `<root>/src-tauri`).
fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap_or(Path::new(".")).to_path_buf()
}

fn emit(app: &AppHandle, step: &str, error: Option<String>) {
    let _ = app.emit("restart-status", RestartStatus { step: step.into(), error });
}

/// Runs `program args` in `dir`; its output goes to the app's terminal.
fn run(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    #[cfg(windows)]
    let mut command = {
        // npm is a .cmd script on Windows.
        let mut c = Command::new("cmd");
        c.arg("/C").arg(program);
        c
    };
    #[cfg(not(windows))]
    let mut command = Command::new(program);
    let status = command.args(args).current_dir(dir).status().map_err(|e| format!("{program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed ({status}); see the app's terminal", args.join(" ")))
    }
}

fn update(app: &AppHandle) -> Result<PathBuf, String> {
    let root = repo();
    if !root.join(".git").exists() {
        return Err(format!("No source repository at {}", root.display()));
    }
    emit(app, "Pulling the latest version…", None);
    // --no-rebase: a fast-forward never rewrites local work, whatever pull.rebase says.
    run(&root, "git", &["pull", "--ff-only", "--no-rebase"])?;
    emit(app, "Building…", None);
    run(&root, "npm", &["run", "desktop:build"])?;
    let binary = root
        .join("src-tauri/target/release")
        .join(format!("poe-batonga{}", std::env::consts::EXE_SUFFIX));
    if !binary.exists() {
        return Err(format!("Build finished but {} is missing", binary.display()));
    }
    Ok(binary)
}

/// Saves what would otherwise be lost, then replaces this process with `binary`.
fn relaunch(app: &AppHandle, binary: &Path) -> Result<(), String> {
    emit(app, "Restarting…", None);
    app.state::<crate::layout::Recorder>().save_now();
    if let Ok(inner) = app.state::<crate::tracker::Tracker>().0.lock() {
        inner.save();
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    #[cfg(unix)]
    {
        // Same process and terminal, so the job's tab keeps showing its output.
        use std::os::unix::process::CommandExt;
        let error = Command::new(binary).args(&args).exec();
        Err(format!("Could not start {}: {error}", binary.display()))
    }
    #[cfg(not(unix))]
    {
        // Only one copy may run: the new one starts once this one has exited.
        let quoted = format!("\"{}\"", binary.display());
        Command::new("cmd")
            .args(["/C", "timeout", "/t", "2", "/nobreak", ">NUL", "&", "start", "\"\"", &quoted])
            .args(&args)
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", binary.display()))?;
        app.exit(0);
        Ok(())
    }
}

/// `tauri dev`: pull, then touch a source file so its watcher rebuilds and restarts the app.
fn dev_restart(app: &AppHandle) -> Result<(), String> {
    let root = repo();
    emit(app, "Pulling the latest version…", None);
    run(&root, "git", &["pull", "--ff-only", "--no-rebase"])?;
    emit(app, "Rebuilding (dev mode)…", None);
    app.state::<crate::layout::Recorder>().save_now();
    if let Ok(inner) = app.state::<crate::tracker::Tracker>().0.lock() {
        inner.save();
    }
    let main = root.join("src-tauri/src/main.rs");
    std::fs::File::options()
        .append(true)
        .open(&main)
        .and_then(|file| file.set_modified(std::time::SystemTime::now()))
        .map_err(|e| format!("Could not touch {}: {e}", main.display()))
}

#[tauri::command]
pub fn restart_latest(app: AppHandle) -> Result<(), String> {
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Already updating".into());
    }
    std::thread::spawn(move || {
        let result = if cfg!(debug_assertions) {
            dev_restart(&app)
        } else {
            update(&app).and_then(|binary| relaunch(&app, &binary))
        };
        if let Err(error) = result {
            eprintln!("Restart: {error}");
            emit(&app, "", Some(error));
        }
        BUSY.store(false, Ordering::SeqCst);
    });
    Ok(())
}
