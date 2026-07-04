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
    fs::create_dir_all(&build).map_err(|e| Status::ToolMissing(format!("cannot create build dir: {e}")))?;
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
            ["\\ref", "\\pageref", "\\eqref", "\\cite", "\\tableofcontents"]
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
    match run_pdflatex(root, ex) {
        Err(s) => return s,
        Ok(_) => {}
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
