//! `latexlings dev-check` — the exercise-author harness (like `rustlings
//! dev check`). Asserts, for every exercise in info.toml:
//!
//! 1. the exercise file and its solution exist, the hint is non-empty
//! 2. the shipped exercise contains the I-AM-NOT-DONE marker;
//!    the solution does not
//! 3. shipped state behaves as declared:
//!    fix   -> fails to compile (unless compiles_as_shipped)
//!    write -> compiles, and its checks do NOT all pass yet
//!    (otherwise the exercise would be solved on arrival)
//! 4. the solution, dropped into a scratch root, verifies as Done
//!    (compiles + passes every check)

use crate::info::{Exercise, Mode, MARKER};
use crate::verify::{verify, Status};
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

struct Failure {
    exercise: String,
    problem: String,
}

/// A valid exercise/category identifier: non-empty and made only of ASCII
/// alphanumerics/underscores — anything else risks unsafe paths (e.g. `..`
/// path traversal) or broken `hint`/`run` lookups by name.
fn is_valid_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every exercise `name` and `dir` must be non-empty and made only of
/// ASCII alphanumerics/underscores — anything else risks unsafe paths or
/// broken `hint`/`run` lookups by name.
fn check_names_and_dirs(exercises: &[Exercise], fail: &mut impl FnMut(&str, String)) {
    for ex in exercises {
        if !is_valid_ident(&ex.name) {
            fail(&ex.name, format!("name `{}` must be non-empty alphanumeric/underscore", ex.name));
        }
        if !is_valid_ident(&ex.dir) {
            fail(&ex.name, format!("dir `{}` must be non-empty alphanumeric/underscore", ex.dir));
        }
    }
}

/// Every `.tex` file under `exercises/` and `solutions/` must be reachable
/// from `info.toml` — otherwise it's dead content nobody runs dev-check
/// against, silently drifting out of sync.
fn check_no_orphan_files(root: &Path, exercises: &[Exercise], fail: &mut impl FnMut(&str, String)) -> Result<()> {
    let known_exercises: HashSet<String> = exercises.iter().map(Exercise::rel_path).collect();
    check_orphans_in(&root.join("exercises"), root, &known_exercises, fail)?;

    let known_solutions: HashSet<String> = exercises.iter().map(Exercise::solution_rel).collect();
    check_orphans_in(&root.join("solutions"), root, &known_solutions, fail)?;
    Ok(())
}

fn check_orphans_in(
    base_dir: &Path,
    root: &Path,
    known: &HashSet<String>,
    fail: &mut impl FnMut(&str, String),
) -> Result<()> {
    if !base_dir.is_dir() {
        return Ok(());
    }
    for entry in walk_tex_files(base_dir)? {
        let rel = entry
            .strip_prefix(root)
            .unwrap_or(&entry)
            .to_string_lossy()
            .replace('\\', "/");
        if !known.contains(&rel) {
            fail("(orphan)", format!("{rel} is not referenced by any exercise in info.toml"));
        }
    }
    Ok(())
}

fn walk_tex_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_tex_files(&path)?);
        } else if path.extension().is_some_and(|e| e == "tex") {
            out.push(path);
        }
    }
    Ok(out)
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

    check_names_and_dirs(exercises, &mut fail);
    check_no_orphan_files(root, exercises, &mut fail)?;

    // Phase 1: cheap sequential checks + scratch-root setup for every exercise.
    let mut solution_jobs = Vec::new();
    let mut scratch_roots = Vec::new();
    for (i, ex) in exercises.iter().enumerate() {
        println!("[{:>3}/{}] {}", i + 1, exercises.len(), ex.name);
        let ex_path = ex.path(root);
        let sol_path = root.join(ex.solution_rel());

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

        let ex_text = match fs::read_to_string(&ex_path) {
            Ok(t) => t,
            Err(e) => {
                fail(&ex.name, format!("cannot read exercise file: {e}"));
                continue;
            }
        };
        let sol_text = match fs::read_to_string(&sol_path) {
            Ok(t) => t,
            Err(e) => {
                fail(&ex.name, format!("cannot read solution file: {e}"));
                continue;
            }
        };
        if !ex_text.contains(MARKER) {
            fail(&ex.name, format!("shipped exercise lacks `% {MARKER}` marker"));
        }
        if sol_text.contains(MARKER) {
            fail(&ex.name, "solution still contains the marker".into());
        }

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

        let sroot = match scratch_root(ex, &sol_text) {
            Ok(r) => r,
            Err(e) => {
                fail(&ex.name, format!("cannot build scratch root: {e}"));
                continue;
            }
        };
        solution_jobs.push(crate::check_all::Job { index: i, root: sroot.clone(), exercise: ex.clone() });
        scratch_roots.push(sroot);
    }

    // Phase 2: parallel solution verification (always cold — no memoization).
    let verifier: crate::check_all::Verifier = std::sync::Arc::new(crate::verify::verify);
    let mut sol_results = crate::check_all::run_blocking(solution_jobs, verifier);
    sol_results.sort_by_key(|r| r.index);
    for r in sol_results {
        let ex = &exercises[r.index];
        if !r.status.is_done() {
            let detail = match &r.status {
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
    }
    for sroot in scratch_roots {
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

/// Scaffold a new exercise + solution `.tex` skeleton under `category`,
/// and print an `[[exercises]]` stanza ready to paste into `info.toml`.
///
/// Deliberately does NOT touch `info.toml` itself: it carries hand-authored
/// section-header comment banners (see the `# ── 00_intro ──` banners at
/// the top of the shipped file) that a naive append/insert would either
/// clobber or land in the wrong section. Printing the stanza for a human to
/// paste keeps that authoring under human control.
pub fn dev_new(root: &Path, category: &str, name: &str, mode: Mode) -> Result<()> {
    if !is_valid_ident(category) {
        bail!("category `{category}` must be non-empty alphanumeric/underscore");
    }
    if !is_valid_ident(name) {
        bail!("name `{name}` must be non-empty alphanumeric/underscore");
    }

    let ex_dir = root.join("exercises").join(category);
    let sol_dir = root.join("solutions").join(category);
    fs::create_dir_all(&ex_dir)?;
    fs::create_dir_all(&sol_dir)?;

    let ex_path = ex_dir.join(format!("{name}.tex"));
    let sol_path = sol_dir.join(format!("{name}.tex"));
    if ex_path.exists() || sol_path.exists() {
        bail!("{name} already exists under {category} — pick a different name");
    }

    let (skeleton, solution, checks_comment): (&str, &str, &str) = match mode {
        Mode::Fix => (
            "% I AM NOT DONE\n\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}\n",
            "\\documentclass{article}\n\\begin{document}\nHello\n\\end{document}\n",
            "",
        ),
        Mode::Write => (
            "% I AM NOT DONE\n\\documentclass{article}\n\\begin{document}\n% write the requested content here\n\\end{document}\n",
            "\\documentclass{article}\n\\begin{document}\nExpected content\n\\end{document}\n",
            "checks = [{ pattern = \"...\", note = \"...\" }]\n",
        ),
    };
    fs::write(&ex_path, skeleton)?;
    fs::write(&sol_path, solution)?;

    println!("created {}", ex_path.display());
    println!("created {}", sol_path.display());
    println!();
    println!("Paste this into info.toml under the `{category}` section:");
    println!();
    println!("[[exercises]]");
    println!("name = \"{name}\"");
    println!("dir = \"{category}\"");
    println!("mode = \"{}\"", mode.label());
    if !checks_comment.is_empty() {
        print!("{checks_comment}");
    }
    println!("hint = \"\"\"");
    println!("Write a hint here.\"\"\"");
    Ok(())
}
