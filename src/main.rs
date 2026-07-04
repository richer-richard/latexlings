mod dev;
mod info;
mod tui;
mod verify;

use anyhow::{bail, Result};
use info::{find_root, load_info, Exercise};
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

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

Workflow: run `latexlings` in one terminal pane, edit the shown .tex file
with vim in another. On every :w it recompiles; fix the file (or write the
requested document), make the checks pass, delete the `% I AM NOT DONE`
line, press n.";

fn find_exercise<'a>(exercises: &'a [Exercise], name: &str) -> Result<&'a Exercise> {
    exercises.iter().find(|e| e.name == name).ok_or_else(|| {
        anyhow::anyhow!("no exercise named `{name}` — try `latexlings list`")
    })
}

fn cmd_run(root: &PathBuf, ex: &Exercise) -> Result<bool> {
    println!("── {} [{}]", ex.rel_path(), ex.mode.label());
    let status = verify::verify(root, ex);
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
    Ok(status.is_done())
}

fn cmd_verify(root: &PathBuf, exercises: &[Exercise]) -> Result<bool> {
    let mut all_ok = true;
    let total = exercises.len();
    for (i, ex) in exercises.iter().enumerate() {
        let status = verify::verify(root, ex);
        let ok = status.is_done();
        all_ok &= ok;
        println!(
            "[{:>3}/{total}] {} {} — {}",
            i + 1,
            if ok { "✓" } else { "✗" },
            ex.rel_path(),
            verify::summarize(&status)
        );
    }
    Ok(all_ok)
}

fn cmd_list(root: &PathBuf, exercises: &[Exercise]) {
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

fn real_main() -> Result<bool> {
    let args: Vec<String> = env::args().skip(1).collect();
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
            tui::run_watch(root, exercises)?;
            Ok(true)
        }
        Some("run") => {
            let name = args.get(1).map(String::as_str);
            let Some(name) = name else { bail!("usage: latexlings run <name>") };
            let ex = find_exercise(&exercises, name)?;
            cmd_run(&root, ex)
        }
        Some("verify") => cmd_verify(&root, &exercises),
        Some("dev-check") => dev::dev_check(&root, &exercises),
        Some("hint") => {
            let Some(name) = args.get(1) else { bail!("usage: latexlings hint <name>") };
            let ex = find_exercise(&exercises, name)?;
            println!("{}", ex.hint);
            Ok(true)
        }
        Some("list") => {
            cmd_list(&root, &exercises);
            Ok(true)
        }
        Some("reset") => {
            let Some(name) = args.get(1) else { bail!("usage: latexlings reset <name>") };
            let ex = find_exercise(&exercises, name)?;
            info::reset(&root, ex)?;
            Ok(true)
        }
        Some("solution") => {
            let Some(name) = args.get(1) else { bail!("usage: latexlings solution <name>") };
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
