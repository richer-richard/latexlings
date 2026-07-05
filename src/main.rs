mod check_all;
mod dev;
mod editor;
mod info;
mod tui;
mod verify;

use anyhow::{bail, Result};
use info::{find_root, load_info, Exercise};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use verify::Status;

const USAGE: &str = "\
latexlings — small exercises for learning LaTeX (inspired by rustlings)

Usage:
  latexlings init [dir]      extract the exercises into a practice directory
                             (default: ./latexlings)
  latexlings [watch]         interactive watch mode — recompiles on save
  latexlings run <name>      verify one exercise, print the result
  latexlings verify          verify every exercise, print a summary
  latexlings hint <name>     show the hint for an exercise
  latexlings list            plain list of all exercises and their status
  latexlings reset <name>    restore an exercise to its original state
  latexlings solution <name> print the reference solution
  latexlings dev-new <category>/<name> [--mode fix|write]
                             scaffold a new exercise + solution skeleton

Flags (watch mode only):
  --edit-cmd <cmd>           open the current exercise with this command
                             instead of auto-detecting an editor
  --no-editor                never auto-open an editor (same as setting
                             LATEXLINGS_NO_EDITOR)

Workflow: run `latexlings` in one terminal pane, edit the shown .tex file
with vim in another. On every :w it recompiles; fix the file (or write the
requested document), make the checks pass, delete the `% I AM NOT DONE`
line, press n.";

fn find_exercise<'a>(exercises: &'a [Exercise], name: &str) -> Result<&'a Exercise> {
    exercises
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| anyhow::anyhow!("no exercise named `{name}` — try `latexlings list`"))
}

fn cmd_run(root: &Path, ex: &Exercise) -> Result<bool> {
    println!("── {} [{}]", ex.rel_path(), ex.mode.label());
    let (status, lints) = verify::verify_with_lints(root, ex);
    match &status {
        verify::Status::Done => println!("✓ done"),
        verify::Status::MarkerPresent => {
            println!("✓ {}", verify::summarize(&status));
        }
        verify::Status::CompileFail(err) => {
            println!("✗ pdflatex failed:\n{err}");
        }
        verify::Status::ChecksFail(notes, excerpt) => {
            println!("✗ compiles, but the rendered output isn't right yet:");
            for n in notes {
                println!("  • {n}");
            }
            println!("  rendered text starts with: {excerpt}");
        }
        verify::Status::ToolMissing(msg) => println!("✗ {msg}"),
    }
    if let Some(lints) = &lints {
        if !lints.notes.is_empty() {
            println!("── chktex notes:");
            for n in &lints.notes {
                println!("  • {n}");
            }
        }
    }
    if status.is_done() {
        mark_done(root, ex)?;
    }
    Ok(status.is_done())
}

/// Persist a completed exercise so watch/list agree with run/verify.
fn mark_done(root: &Path, ex: &Exercise) -> Result<()> {
    let mut done = info::load_done(root);
    let mtime = std::fs::metadata(ex.path(root))
        .and_then(|m| m.modified())
        .ok()
        .map(info::truncate_to_secs);
    let previous_mtime = done.mtime(&ex.name);
    let is_new = done.insert(ex.name.clone(), mtime);
    if is_new || previous_mtime != mtime {
        info::save_done(root, &done)?;
    }
    Ok(())
}

/// Print one exercise's result and persist `done` immediately if its
/// recorded state actually changed. Incremental (rather than batched-at-end)
/// persistence means an interruption mid-sweep only risks losing the one
/// exercise in flight, not the whole run's progress — and skipping the save
/// whenever nothing changed (the common case: unchanged cache hits) keeps
/// this from regressing to a write-on-every-mutation cost like rustlings'.
/// Also clears a previously-Done exercise's stale entry when a fresh
/// re-verification now fails, so a regression is never left recorded done.
fn report_and_persist(
    root: &Path,
    exercises: &[Exercise],
    done: &mut info::DoneState,
    index: usize,
    status: Status,
    total: usize,
) -> Result<bool> {
    let ex = &exercises[index];
    let ok = status.is_done();
    let changed = if ok {
        let mtime = std::fs::metadata(ex.path(root))
            .and_then(|m| m.modified())
            .ok()
            .map(info::truncate_to_secs);
        let previous_mtime = done.mtime(&ex.name);
        let is_new = done.insert(ex.name.clone(), mtime);
        is_new || previous_mtime != mtime
    } else {
        done.remove(&ex.name)
    };
    if changed {
        info::save_done(root, done)?;
    }
    println!(
        "[{:>3}/{total}] {} {} — {}",
        index + 1,
        if ok { "✓" } else { "✗" },
        ex.rel_path(),
        verify::summarize(&status)
    );
    Ok(ok)
}

fn cmd_verify(root: &Path, exercises: &[Exercise]) -> Result<bool> {
    let mut done = info::load_done(root);
    let (jobs, mut cached) = check_all::plan_sweep(root, exercises, &done);
    let total = exercises.len();
    let mut all_ok = true;

    // Cache hits need no compile — report them immediately, in order.
    cached.sort_by_key(|(i, _)| *i);
    for (i, status) in cached {
        all_ok &= report_and_persist(root, exercises, &mut done, i, status, total)?;
    }

    // Fresh jobs stream in as each finishes (not necessarily input order),
    // so a slow/hung exercise no longer blocks all output for the sweep.
    // verify_respecting_strict_chktex keeps the bulk path fast for the
    // common (non-strict_chktex) case while still honoring strict_chktex
    // for the rare exercise that sets it, instead of silently ignoring it.
    let verifier: check_all::Verifier = Arc::new(verify::verify_respecting_strict_chktex);
    let mut stream_err: Option<anyhow::Error> = None;
    check_all::run_streaming(jobs, verifier, |result| {
        if stream_err.is_some() {
            return;
        }
        match report_and_persist(
            root,
            exercises,
            &mut done,
            result.index,
            result.status,
            total,
        ) {
            Ok(ok) => all_ok &= ok,
            Err(e) => stream_err = Some(e),
        }
    });
    if let Some(e) = stream_err {
        return Err(e);
    }

    Ok(all_ok)
}

fn cmd_list(root: &Path, exercises: &[Exercise]) {
    let done = info::load_done(root);
    for (i, ex) in exercises.iter().enumerate() {
        println!(
            "{:>3} {} {:<26} {:<22} {}",
            i + 1,
            if done.contains(&ex.name) { "✓" } else { "·" },
            ex.name,
            ex.dir,
            ex.mode.label()
        );
    }
    println!("\n{}/{} done", done.len(), exercises.len());
}

/// Pulls `--edit-cmd <value>` and `--no-editor` out of `args`, wherever they
/// appear, and builds the resulting `EditorConfig`. Removing them up front
/// (rather than just reading past them) keeps subcommand dispatch (`args
/// .first()`) and positional lookups (`args.get(1)`, …) working regardless
/// of where the caller places these flags — including with no subcommand at
/// all, e.g. `latexlings --no-editor`.
fn take_editor_flags(args: &mut Vec<String>) -> editor::EditorConfig {
    let edit_cmd = args.iter().position(|a| a == "--edit-cmd").and_then(|i| {
        args.remove(i);
        // Only consume the next token as --edit-cmd's value if it doesn't
        // itself look like a flag (e.g. `--edit-cmd --no-editor` with no
        // command supplied must not swallow --no-editor as data).
        (i < args.len() && !args[i].starts_with("--")).then(|| args.remove(i))
    });
    let no_editor = args
        .iter()
        .position(|a| a == "--no-editor")
        .map(|i| args.remove(i))
        .is_some()
        || env::var("LATEXLINGS_NO_EDITOR").is_ok();
    editor::EditorConfig {
        edit_cmd,
        disabled: no_editor,
    }
}

fn real_main() -> Result<bool> {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let editor_config = take_editor_flags(&mut args);
    let cmd = args.first().map(String::as_str);

    match cmd {
        Some("init") => {
            let target = args
                .get(1)
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("latexlings"));
            info::init(&target)?;
            return Ok(true);
        }
        Some("help") | Some("--help") | Some("-h") => {
            println!("{USAGE}");
            return Ok(true);
        }
        Some("--version") | Some("-V") => {
            println!("latexlings {}", env!("CARGO_PKG_VERSION"));
            return Ok(true);
        }
        _ => {}
    }

    let root = find_root()?;
    let exercises = load_info(&root)?.exercises;
    if exercises.is_empty() {
        bail!("info.toml lists no exercises");
    }

    match cmd {
        None | Some("watch") => {
            tui::run_watch(root, exercises, editor_config)?;
            Ok(true)
        }
        Some("run") => {
            let name = args.get(1).map(String::as_str);
            let Some(name) = name else {
                bail!("usage: latexlings run <name>")
            };
            let ex = find_exercise(&exercises, name)?;
            cmd_run(&root, ex)
        }
        Some("verify") => cmd_verify(&root, &exercises),
        Some("dev-check") => dev::dev_check(&root, &exercises),
        Some("dev-new") => {
            let spec = args.get(1).ok_or_else(|| {
                anyhow::anyhow!("usage: latexlings dev-new <category>/<name> [--mode fix|write]")
            })?;
            let (category, name) = spec.split_once('/').ok_or_else(|| {
                anyhow::anyhow!("expected <category>/<name>, e.g. 16_something/foo1")
            })?;
            let mode = match args
                .iter()
                .position(|a| a == "--mode")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
            {
                Some("write") => info::Mode::Write,
                _ => info::Mode::Fix,
            };
            dev::dev_new(&root, category, name, mode)?;
            Ok(true)
        }
        Some("hint") => {
            let Some(name) = args.get(1) else {
                bail!("usage: latexlings hint <name>")
            };
            let ex = find_exercise(&exercises, name)?;
            println!("{}", ex.hint);
            Ok(true)
        }
        Some("list") => {
            cmd_list(&root, &exercises);
            Ok(true)
        }
        Some("reset") => {
            let Some(name) = args.get(1) else {
                bail!("usage: latexlings reset <name>")
            };
            let ex = find_exercise(&exercises, name)?;
            info::reset(&root, ex)?;
            Ok(true)
        }
        Some("solution") => {
            let Some(name) = args.get(1) else {
                bail!("usage: latexlings solution <name>")
            };
            let ex = find_exercise(&exercises, name)?;
            let disk = root.join(ex.solution_rel());
            let text = if disk.is_file() {
                std::fs::read_to_string(&disk)?
            } else {
                info::EMBEDDED_SOLUTIONS
                    .get_file(format!("{}/{}.tex", ex.dir, ex.name))
                    .map(|f| String::from_utf8_lossy(f.contents()).into_owned())
                    .ok_or_else(|| anyhow::anyhow!("no solution found for {name}"))?
            };
            println!("{text}");
            Ok(true)
        }
        Some(other) => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_cmd_does_not_swallow_a_following_flag_as_its_value() {
        let mut args = vec!["--edit-cmd".to_string(), "--no-editor".to_string()];
        let config = take_editor_flags(&mut args);
        assert_eq!(
            config.edit_cmd, None,
            "--no-editor must not become --edit-cmd's value"
        );
        assert!(
            config.disabled,
            "--no-editor must still be recognized and applied"
        );
        assert!(args.is_empty(), "both flags should be consumed from args");
    }

    #[test]
    fn edit_cmd_still_takes_a_real_value() {
        let mut args = vec![
            "--edit-cmd".to_string(),
            "vim".to_string(),
            "run".to_string(),
        ];
        let config = take_editor_flags(&mut args);
        assert_eq!(config.edit_cmd, Some("vim".to_string()));
        assert!(!config.disabled);
        assert_eq!(args, vec!["run".to_string()]);
    }

    #[test]
    fn no_editor_flag_alone_disables_without_edit_cmd() {
        let mut args = vec!["--no-editor".to_string()];
        let config = take_editor_flags(&mut args);
        assert_eq!(config.edit_cmd, None);
        assert!(config.disabled);
        assert!(args.is_empty());
    }
}
