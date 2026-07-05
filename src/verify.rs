//! Compile an exercise with pdflatex, extract errors from the log, and run
//! `pdftotext` content checks for writing assignments.

use crate::info::{Exercise, Mode, MARKER};
use anyhow::Result;
use regex::Regex;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug)]
pub enum Status {
    /// Compiles, checks pass, marker removed.
    Done,
    /// Compiles and passes checks, but `% I AM NOT DONE` is still present.
    MarkerPresent,
    /// pdflatex failed; payload is the extracted error excerpt.
    CompileFail(String),
    /// Compiles, but the rendered text does not satisfy the checks.
    /// Payload: failed-check notes, plus an excerpt of the rendered text.
    ChecksFail(Vec<String>, String),
    /// A required external tool is missing.
    ToolMissing(String),
}

impl Status {
    pub fn is_done(&self) -> bool {
        matches!(self, Status::Done)
    }
}

fn build_dir(root: &Path, ex: &Exercise) -> PathBuf {
    root.join("build").join(&ex.name)
}

/// Extract the interesting part of pdflatex's output: from the first error
/// line (`!` or `file.tex:NN:`) onward, capped.
fn extract_errors(stdout: &str) -> String {
    let err_re = Regex::new(r"^[^ ]*\.tex:\d+:").unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with('!') || err_re.is_match(l));
    match start {
        Some(idx) => {
            let end = (idx + 18).min(lines.len());
            lines[idx..end].join("\n")
        }
        None => {
            // no classic error line — show the tail, something went wrong there
            let tail = lines.len().saturating_sub(12);
            lines[tail..].join("\n")
        }
    }
}

fn run_pdflatex(root: &Path, ex: &Exercise) -> Result<Status, Status> {
    let build = build_dir(root, ex);
    fs::create_dir_all(&build)
        .map_err(|e| Status::ToolMissing(format!("cannot create build dir: {e}")))?;
    let output = Command::new("pdflatex")
        .current_dir(root)
        .arg("-interaction=nonstopmode")
        .arg("-halt-on-error")
        .arg("-file-line-error")
        .arg("-output-directory")
        .arg(&build)
        .arg(ex.rel_path())
        .output();
    let output = match output {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(Status::ToolMissing(
                "pdflatex not found on PATH.\nInstall a TeX distribution:\n  \
                 macOS:  brew install --cask mactex-no-gui\n  \
                 Debian: sudo apt install texlive-latex-extra"
                    .into(),
            ))
        }
        Err(e) => return Err(Status::ToolMissing(format!("failed to run pdflatex: {e}"))),
        Ok(o) => o,
    };
    if output.status.success() {
        Ok(Status::Done) // provisional; caller refines
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(Status::CompileFail(extract_errors(&stdout)))
    }
}

/// Does this source need a second pass for references/contents to resolve?
fn needs_second_pass(root: &Path, ex: &Exercise) -> bool {
    fs::read_to_string(ex.path(root))
        .map(|s| {
            [
                "\\ref",
                "\\pageref",
                "\\eqref",
                "\\cite",
                "\\tableofcontents",
            ]
            .iter()
            .any(|n| s.contains(n))
        })
        .unwrap_or(false)
}

fn rendered_text(root: &Path, ex: &Exercise) -> Result<String, Status> {
    let pdf = build_dir(root, ex).join(format!("{}.pdf", ex.name));
    let output = Command::new("pdftotext")
        .arg("-q")
        .arg(&pdf)
        .arg("-")
        .output();
    let output = match output {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(Status::ToolMissing(
                "pdftotext not found — writing exercises check the rendered PDF text.\n\
                 Install poppler:\n  macOS:  brew install poppler\n  Debian: sudo apt install poppler-utils"
                    .into(),
            ))
        }
        Err(e) => return Err(Status::ToolMissing(format!("failed to run pdftotext: {e}"))),
        Ok(o) => o,
    };
    let raw = String::from_utf8_lossy(&output.stdout);
    // normalize: any whitespace run -> single space, so checks are layout-proof
    Ok(raw.split_whitespace().collect::<Vec<_>>().join(" "))
}

pub fn verify(root: &Path, ex: &Exercise) -> Status {
    if let Err(s) = run_pdflatex(root, ex) {
        return s;
    }
    if needs_second_pass(root, ex) {
        if let Err(s) = run_pdflatex(root, ex) {
            return s;
        }
    }
    if ex.mode == Mode::Write && !ex.checks.is_empty() {
        let text = match rendered_text(root, ex) {
            Ok(t) => t,
            Err(s) => return s,
        };
        let mut failed = Vec::new();
        for check in &ex.checks {
            match Regex::new(&check.pattern) {
                Ok(re) => {
                    if !re.is_match(&text) {
                        failed.push(check.note.clone());
                    }
                }
                Err(e) => failed.push(format!("(bad check pattern `{}`: {e})", check.pattern)),
            }
        }
        if !failed.is_empty() {
            let excerpt: String = text.chars().take(360).collect();
            return Status::ChecksFail(failed, excerpt);
        }
    }
    if ex.has_marker(root) {
        Status::MarkerPresent
    } else {
        Status::Done
    }
}

/// One-line human summary, used by `run`/`verify` plain output.
pub fn summarize(status: &Status) -> String {
    match status {
        Status::Done => "done".into(),
        Status::MarkerPresent => format!("passing — remove the `% {MARKER}` line to finish"),
        Status::CompileFail(_) => "compile error".into(),
        Status::ChecksFail(notes, _) => format!("output checks failed ({})", notes.len()),
        Status::ToolMissing(_) => "missing tool".into(),
    }
}

pub struct LintResult {
    pub notes: Vec<String>,
}

/// Extract `Warning ...` lines from chktex's stdout, dropping the trailing
/// summary line and blank lines.
fn parse_chktex_output(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|l| l.starts_with("Warning"))
        .map(str::to_string)
        .collect()
}

/// Best-effort chktex pass: `None` if the tool isn't installed or the
/// exercise doesn't compile (nothing meaningful to lint yet). Never treated
/// as a hard failure the way a missing pdflatex/pdftotext is.
pub fn run_chktex(root: &Path, ex: &Exercise) -> Option<LintResult> {
    let output = Command::new("chktex")
        .arg("-q")
        .current_dir(root)
        .arg(ex.rel_path())
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(LintResult {
        notes: parse_chktex_output(&stdout),
    })
}

/// Runs the normal verify pipeline, then — only if it compiled — best-effort
/// chktex. If `ex.strict_chktex` and chktex produced notes, downgrades a
/// would-be-Done/MarkerPresent status to ChecksFail so lint issues block
/// completion the same way a failing content check does. If the exercise
/// already failed its content checks (`ChecksFail`), the chktex notes are
/// merged into the existing note list instead of replacing it, so a
/// double-failure (content checks AND strict chktex) doesn't lose the
/// original diagnostic detail.
pub fn verify_with_lints(root: &Path, ex: &Exercise) -> (Status, Option<LintResult>) {
    let status = verify(root, ex);
    if matches!(status, Status::CompileFail(_) | Status::ToolMissing(_)) {
        return (status, None);
    }
    let lints = run_chktex(root, ex);
    let has_notes = lints.as_ref().map(|l| !l.notes.is_empty()).unwrap_or(false);
    if ex.strict_chktex && has_notes {
        let chktex_notes = lints.as_ref().unwrap().notes.clone();
        let new_status = match status {
            Status::ChecksFail(mut notes, excerpt) => {
                notes.extend(chktex_notes.into_iter().map(|n| format!("chktex: {n}")));
                Status::ChecksFail(notes, excerpt)
            }
            _ => Status::ChecksFail(chktex_notes, String::new()),
        };
        return (new_status, lints);
    }
    (status, lints)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chktex_notes_extracts_warning_lines() {
        let sample = "\
Warning 1 in ./foo.tex line 12: Command terminated with space.\n\
Warning 24 in ./foo.tex line 20: Delete this space to maintain correctness.\n\
\n\
ChkTeX: 2 warnings printed; 0 errors printed.\n";
        let notes = parse_chktex_output(sample);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].contains("line 12"));
        assert!(notes[1].contains("line 20"));
    }

    #[test]
    fn parse_chktex_notes_on_clean_output_is_empty() {
        let sample = "ChkTeX: No warnings printed.\n";
        assert!(parse_chktex_output(sample).is_empty());
    }
}
