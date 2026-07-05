//! Editor-agnostic auto-open: on every exercise change, get the file in
//! front of the learner using whatever editor mechanism is available —
//! an explicit `--edit-cmd`, VS Code, a reused tmux/Zellij pane, or a plain
//! `$EDITOR` fallback. Best-effort and non-blocking: any failure here is
//! silently swallowed, it never affects verification.

use std::env;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

#[derive(Clone, Default)]
pub struct EditorConfig {
    pub edit_cmd: Option<String>,
    pub disabled: bool,
}

/// Best-effort, non-blocking: spawns a background thread and returns
/// immediately. Never panics; any failure is silently dropped.
pub fn open(config: &EditorConfig, path: &Path) {
    if config.disabled {
        return;
    }
    let path = path.to_path_buf();
    let config = config.clone();
    std::thread::spawn(move || {
        let _ = try_open(&config, &path);
    });
}

fn try_open(config: &EditorConfig, path: &Path) -> Option<()> {
    if let Some(cmd) = &config.edit_cmd {
        return run_edit_cmd(cmd, path);
    }
    if env::var("TERM_PROGRAM").as_deref() == Ok("vscode") {
        for bin in ["code", "codium"] {
            if which(bin) {
                return run_edit_cmd(bin, path);
            }
        }
    }
    if env::var("ZELLIJ").is_ok() && which("zellij") {
        return open_in_zellij_pane(path);
    }
    if env::var("TMUX").is_ok() && which("tmux") {
        return open_in_tmux_pane(path);
    }
    let editor = env::var("EDITOR").ok()?;
    run_edit_cmd(&editor, path)
}

/// The `terminal_<id>` pane `zellij action edit` most recently created, so
/// the next call can close it before opening a new one instead of leaking
/// a fresh floating pane per exercise. Holding the lock for the whole
/// close-then-edit sequence also serializes concurrent `editor::open` calls
/// (e.g. from rapid navigation) against each other.
static LAST_ZELLIJ_PANE: Mutex<Option<String>> = Mutex::new(None);

fn open_in_zellij_pane(path: &Path) -> Option<()> {
    let mut last_pane = LAST_ZELLIJ_PANE.lock().ok()?;
    if let Some(pane_id) = last_pane.take() {
        // `close-pane -p <id>` targets a specific pane directly — no need
        // to focus it first, unlike `close-pane`'s focused-pane default.
        let _ = Command::new("zellij")
            .args(["action", "close-pane", "-p", &pane_id])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    // `zellij action edit <path>` prints the created pane's ID
    // (`terminal_<id>`) to stdout — capture it via `.output()` (which pipes
    // stdout/stderr regardless of prior redirection) so the next call can
    // close exactly this pane.
    let out = Command::new("zellij")
        .args(["action", "edit"])
        .arg(path)
        .output()
        .ok()?;
    let new_pane_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !new_pane_id.is_empty() {
        *last_pane = Some(new_pane_id);
    }
    Some(())
}

fn run_edit_cmd(cmd: &str, path: &Path) -> Option<()> {
    let mut parts = shlex::split(cmd)?;
    if parts.is_empty() {
        return None;
    }
    let program = parts.remove(0);
    let _ = Command::new(program)
        .args(parts)
        .arg(path)
        // This path (an explicit --edit-cmd, VS Code, or the plain $EDITOR
        // fallback with no multiplexer detected) is spawned from a
        // background thread while the TUI's own event loop keeps reading
        // the same stdin on the main thread. Redirecting stdin to null
        // means a full-screen terminal editor spawned this way can't render
        // (a pre-existing, documented limitation of running without
        // tmux/Zellij) — but it can no longer consume keystrokes intended
        // for the TUI either, which is the actually dangerous failure mode
        // this closes off.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Some(())
}

const PANE_TITLE: &str = "latexlings-editor";

/// Serializes the whole list-panes → decide → create-or-reuse sequence in
/// `open_in_tmux_pane`. Without it, two overlapping `editor::open` calls
/// (e.g. rapid exercise navigation) could each see no existing pane yet and
/// both `split-window`, leaking a duplicate instead of reusing one.
static TMUX_PANE_LOCK: Mutex<()> = Mutex::new(());

fn open_in_tmux_pane(path: &Path) -> Option<()> {
    let _guard = TMUX_PANE_LOCK.lock().ok()?;
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    // `-s` scopes the listing to the current session only. `open_in_tmux_pane`
    // only ever creates panes in the session this process is attached to, so
    // `-s` is sufficient to find a previously-created pane — using `-a`
    // (whole server) would risk matching a same-titled pane left over from an
    // unrelated tmux session.
    let list = Command::new("tmux")
        .args(["list-panes", "-s", "-F", "#{pane_id} #{pane_title}"])
        .output()
        .ok()?;
    let list = String::from_utf8_lossy(&list.stdout);
    let existing_pane = list
        .lines()
        .find(|l| l.ends_with(PANE_TITLE))
        .and_then(|l| l.split_whitespace().next());

    let path_str = path.to_str()?;
    // `send-keys` types this string as literal keystrokes into whatever
    // shell is running in the target pane — unlike the argv-based spawns
    // used elsewhere in this file, that shell (not tmux) interprets any
    // metacharacters the string contains. Shell-quote the path so a
    // practice root containing `&`, `()`, spaces, etc. can't be
    // reinterpreted as shell syntax.
    let quoted_path = shlex::try_quote(path_str).ok()?;
    let edit_command = format!("{editor} {quoted_path}");
    if let Some(pane_id) = existing_pane {
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", pane_id, "C-c"])
            .status();
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", pane_id, &edit_command, "Enter"])
            .status();
    } else {
        let out = Command::new("tmux")
            .args(["split-window", "-h", "-P", "-F", "#{pane_id}"])
            .output()
            .ok()?;
        let pane_id = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let _ = Command::new("tmux")
            .args(["select-pane", "-t", &pane_id, "-T", PANE_TITLE])
            .status();
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", &pane_id, &edit_command, "Enter"])
            .status();
    }
    Some(())
}

fn which(bin: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}
