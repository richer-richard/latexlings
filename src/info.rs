//! Exercise metadata, embedded assets, practice-directory discovery, and
//! progress state.

use anyhow::{bail, Context, Result};
use include_dir::{include_dir, Dir, DirEntry};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Exercises and solutions are compiled into the binary so that
/// `cargo install latexlings && latexlings init` works with no checkout.
pub static EMBEDDED_EXERCISES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/exercises");
pub static EMBEDDED_SOLUTIONS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/solutions");
pub const EMBEDDED_INFO: &str = include_str!("../info.toml");

/// An exercise is finished only once this marker line is deleted.
pub const MARKER: &str = "I AM NOT DONE";

#[derive(Deserialize, Clone, Debug)]
pub struct Check {
    /// Regex matched against whitespace-normalized `pdftotext` output.
    pub pattern: String,
    /// Human explanation shown when the pattern does not match.
    pub note: String,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Broken file: make it compile.
    Fix,
    /// Typesetting assignment: skeleton compiles, you write the content;
    /// `checks` verify the rendered PDF text.
    Write,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Fix => "fix",
            Mode::Write => "write",
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct Exercise {
    pub name: String,
    pub dir: String,
    pub mode: Mode,
    pub hint: String,
    #[serde(default)]
    pub checks: Vec<Check>,
    /// fix-mode only: true when the shipped file intentionally compiles
    /// (e.g. intro1). dev-check uses this to assert brokenness.
    #[serde(default)]
    pub compiles_as_shipped: bool,
}

impl Exercise {
    pub fn rel_path(&self) -> String {
        format!("exercises/{}/{}.tex", self.dir, self.name)
    }
    pub fn path(&self, root: &Path) -> PathBuf {
        root.join(self.rel_path())
    }
    pub fn solution_rel(&self) -> String {
        format!("solutions/{}/{}.tex", self.dir, self.name)
    }
    pub fn has_marker(&self, root: &Path) -> bool {
        fs::read_to_string(self.path(root))
            .map(|s| s.contains(MARKER))
            .unwrap_or(false)
    }
}

#[derive(Deserialize)]
pub struct Info {
    pub exercises: Vec<Exercise>,
}

pub fn load_info(root: &Path) -> Result<Info> {
    let disk = root.join("info.toml");
    let text = if disk.is_file() {
        fs::read_to_string(&disk).context("reading info.toml")?
    } else {
        EMBEDDED_INFO.to_string()
    };
    let info: Info = toml::from_str(&text).context("parsing info.toml")?;
    let mut seen = BTreeSet::new();
    for ex in &info.exercises {
        if !seen.insert(ex.name.clone()) {
            bail!("duplicate exercise name in info.toml: {}", ex.name);
        }
    }
    Ok(info)
}

/// Walk up from the current directory to find a practice dir (created by
/// `latexlings init`) or the development checkout itself.
pub fn find_root() -> Result<PathBuf> {
    let mut dir = env::current_dir()?;
    loop {
        if dir.join("exercises").is_dir()
            && (dir.join(".latexlings-root").is_file() || dir.join("info.toml").is_file())
        {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "not inside a latexlings directory.\n\
                 Run `latexlings init`, then `cd latexlings` and try again."
            );
        }
    }
}

// ---------------------------------------------------------------------------
// progress state: one exercise name per line in .latexlings-state.txt
// ---------------------------------------------------------------------------

fn state_path(root: &Path) -> PathBuf {
    root.join(".latexlings-state.txt")
}

pub fn load_done(root: &Path) -> BTreeSet<String> {
    fs::read_to_string(state_path(root))
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_done(root: &Path, done: &BTreeSet<String>) -> Result<()> {
    let mut text: String = done.iter().map(|n| format!("{n}\n")).collect();
    text.insert_str(
        0,
        "# latexlings progress — safe to delete if you want to start over\n",
    );
    fs::write(state_path(root), text).context("writing progress state")
}

// ---------------------------------------------------------------------------
// init / reset from embedded assets
// ---------------------------------------------------------------------------

fn extract_dir(dir: &Dir, base: &Path) -> Result<()> {
    for entry in dir.entries() {
        match entry {
            DirEntry::Dir(sub) => {
                fs::create_dir_all(base.join(sub.path()))?;
                extract_dir(sub, base)?;
            }
            DirEntry::File(file) => {
                let dest = base.join(file.path());
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(dest, file.contents())?;
            }
        }
    }
    Ok(())
}

pub fn init(target: &Path) -> Result<()> {
    if target.exists() && target.read_dir()?.next().is_some() {
        bail!("{} already exists and is not empty", target.display());
    }
    fs::create_dir_all(target)?;
    extract_dir(&EMBEDDED_EXERCISES, &target.join("exercises"))?;
    extract_dir(&EMBEDDED_SOLUTIONS, &target.join("solutions"))?;
    fs::write(
        target.join(".latexlings-root"),
        "This file marks a latexlings practice directory. Keep it.\n",
    )?;
    fs::write(target.join(".gitignore"), "build/\n.latexlings-state.txt\n")?;
    println!("initialized latexlings practice directory: {}", target.display());
    println!();
    println!("    cd {}", target.display());
    println!("    latexlings        # start the watch TUI");
    println!();
    println!("Exercises live in exercises/. Edit them with vim in another pane;");
    println!("latexlings recompiles on every save.");
    Ok(())
}

pub fn reset(root: &Path, ex: &Exercise) -> Result<()> {
    let rel = format!("{}/{}.tex", ex.dir, ex.name);
    let file = EMBEDDED_EXERCISES
        .get_file(&rel)
        .with_context(|| format!("no embedded original for {rel}"))?;
    fs::write(ex.path(root), file.contents())?;
    let mut done = load_done(root);
    if done.remove(&ex.name) {
        save_done(root, &done)?;
    }
    println!("reset {}", ex.rel_path());
    Ok(())
}
