//! `latexlings dev-check` — the exercise-author harness (like `rustlings
//! dev check`). Asserts, for every exercise in info.toml:
//!
//!   1. the exercise file and its solution exist, the hint is non-empty
//!   2. the shipped exercise contains the I-AM-NOT-DONE marker;
//!      the solution does not
//!   3. shipped state behaves as declared:
//!        fix   -> fails to compile (unless compiles_as_shipped)
//!        write -> compiles, and its checks do NOT all pass yet
//!                 (otherwise the exercise would be solved on arrival)
//!   4. the solution, dropped into a scratch root, verifies as Done
//!      (compiles + passes every check)

use crate::info::{Exercise, Mode, MARKER};
use crate::verify::{verify, Status};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

struct Failure {
    exercise: String,
    problem: String,
}

fn scratch_root(ex: &Exercise, solution_text: &str) -> Result<PathBuf> {
    let root = std::env::temp_dir().join(format!(
        "latexlings-devcheck-{}-{}",
        std::process::id(),
        ex.name
    ));
    let exdir = root.join("exercises").join(&ex.dir);
    fs::create_dir_all(&exdir)?;
    fs::write(exdir.join(format!("{}.tex", ex.name)), solution_text)?;
    Ok(root)
}

pub fn dev_check(root: &Path, exercises: &[Exercise]) -> Result<bool> {
    let mut failures: Vec<Failure> = Vec::new();
    let mut fail = |name: &str, problem: String| {
        println!("  ✗ {name}: {problem}");
        failures.push(Failure { exercise: name.into(), problem });
    };

    for (i, ex) in exercises.iter().enumerate() {
        println!("[{:>3}/{}] {}", i + 1, exercises.len(), ex.name);
        let ex_path = ex.path(root);
        let sol_path = root.join(ex.solution_rel());

        // 1. files + hint
        if !ex_path.is_file() {
            fail(&ex.name, format!("missing exercise file {}", ex.rel_path()));
            continue;
        }
        if !sol_path.is_file() {
            fail(&ex.name, format!("missing solution file {}", ex.solution_rel()));
            continue;
        }
        if ex.hint.trim().is_empty() {
            fail(&ex.name, "empty hint".into());
        }
        if ex.mode == Mode::Write && ex.checks.is_empty() {
            fail(&ex.name, "write-mode exercise with no checks".into());
        }

        // 2. marker discipline
        let ex_text = fs::read_to_string(&ex_path)?;
        let sol_text = fs::read_to_string(&sol_path)?;
        if !ex_text.contains(MARKER) {
            fail(&ex.name, format!("shipped exercise lacks `% {MARKER}` marker"));
        }
        if sol_text.contains(MARKER) {
            fail(&ex.name, "solution still contains the marker".into());
        }

        // 3. shipped-state behavior
        let shipped = verify(root, ex);
        match (ex.mode, &shipped) {
            (Mode::Fix, Status::CompileFail(_)) if !ex.compiles_as_shipped => {}
            (Mode::Fix, Status::MarkerPresent) if ex.compiles_as_shipped => {}
            (Mode::Fix, other) => fail(
                &ex.name,
                format!(
                    "shipped fix exercise should {} but got: {}",
                    if ex.compiles_as_shipped { "compile (marker gate only)" } else { "FAIL to compile" },
                    crate::verify::summarize(other)
                ),
            ),
            (Mode::Write, Status::ChecksFail(..)) => {}
            (Mode::Write, other) => fail(
                &ex.name,
                format!(
                    "shipped write exercise should compile but fail its checks; got: {}",
                    crate::verify::summarize(other)
                ),
            ),
        }

        // 4. solution verifies as Done in a scratch root
        let sroot = scratch_root(ex, &sol_text).context("building scratch root")?;
        let sol_status = verify(&sroot, ex);
        if !sol_status.is_done() {
            let detail = match &sol_status {
                Status::CompileFail(e) => {
                    let head: String = e.lines().take(3).collect::<Vec<_>>().join(" | ");
                    format!("solution failed to compile: {head}")
                }
                Status::ChecksFail(notes, excerpt) => format!(
                    "solution failed checks: {} (text: {excerpt})",
                    notes.join("; ")
                ),
                other => format!("solution not Done: {}", crate::verify::summarize(other)),
            };
            fail(&ex.name, detail);
        }
        let _ = fs::remove_dir_all(&sroot);
    }

    println!();
    if failures.is_empty() {
        println!("dev-check: all {} exercises pass ✓", exercises.len());
        Ok(true)
    } else {
        println!("dev-check: {} problem(s):", failures.len());
        for f in &failures {
            println!("  {} — {}", f.exercise, f.problem);
        }
        Ok(false)
    }
}
