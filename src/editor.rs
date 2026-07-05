//! Editor-agnostic auto-open: on every exercise change, get the file in
//! front of the learner using whatever editor mechanism is available —
//! an explicit `--edit-cmd`, VS Code, a reused tmux/Zellij pane, or a plain
//! `$EDITOR` fallback. Best-effort and non-blocking: any failure here is
//! silently swallowed, it never affects verification.

use std::env;
use std::path::Path;
use std::process::{Command, Stdio};

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
        let _ = Command::new("zellij")
            .args(["action", "edit"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return Some(());
    }
    if env::var("TMUX").is_ok() && which("tmux") {
        return open_in_tmux_pane(path);
    }
    let editor = env::var("EDITOR").ok()?;
    run_edit_cmd(&editor, path)
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    Some(())
}

const PANE_TITLE: &str = "latexlings-editor";

fn open_in_tmux_pane(path: &Path) -> Option<()> {
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
    if let Some(pane_id) = existing_pane {
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", pane_id, "C-c"])
            .status();
        let _ = Command::new("tmux")
            .args([
                "send-keys",
                "-t",
                pane_id,
                &format!("{editor} {path_str}"),
                "Enter",
            ])
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
            .args([
                "send-keys",
                "-t",
                &pane_id,
                &format!("{editor} {path_str}"),
                "Enter",
            ])
            .status();
    }
    Some(())
}

fn which(bin: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|p| p.join(bin).is_file()))
        .unwrap_or(false)
}
