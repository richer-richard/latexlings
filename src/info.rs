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
    /// When true, non-empty chktex output blocks `Done` the same way a
    /// failing check does. Default false: chktex output is informational only.
    #[serde(default)]
    pub strict_chktex: bool,
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
// progress state: one exercise name per line in .latexlings-state.txt,
// optionally suffixed with a tab and the unix-epoch-seconds mtime the
// exercise's source had when it was last verified Done (used by check_all's
// memoization). A line with no tab has no cached mtime and is always
// reverified on the next sweep.
// ---------------------------------------------------------------------------

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn state_path(root: &Path) -> PathBuf {
    root.join(".latexlings-state.txt")
}

#[derive(Default, Clone, Debug)]
pub struct DoneState {
    entries: BTreeMap<String, Option<SystemTime>>,
}

impl DoneState {
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub fn mtime(&self, name: &str) -> Option<SystemTime> {
        self.entries.get(name).copied().flatten()
    }

    /// Returns true if `name` was not already tracked.
    pub fn insert(&mut self, name: String, mtime: Option<SystemTime>) -> bool {
        let is_new = !self.entries.contains_key(&name);
        self.entries.insert(name, mtime);
        is_new
    }

    pub fn remove(&mut self, name: &str) -> bool {
        self.entries.remove(name).is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // Kept to satisfy clippy's len_without_is_empty convention on `len`; no
    // caller needs it yet, so allow the resulting dead_code warning here.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn parse_mtime_secs(s: &str) -> Option<SystemTime> {
    s.parse::<u64>()
        .ok()
        .map(|secs| UNIX_EPOCH + std::time::Duration::from_secs(secs))
}

fn format_mtime_secs(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Truncate a `SystemTime` to whole-second resolution, matching what
/// `.latexlings-state.txt` persists — so a value compared against one
/// that has round-tripped through `load_done`/`save_done` (or one that
/// hasn't yet) always compares at the same precision.
pub fn truncate_to_secs(t: SystemTime) -> SystemTime {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    UNIX_EPOCH + std::time::Duration::from_secs(secs)
}

pub fn load_done(root: &Path) -> DoneState {
    let mut state = DoneState::default();
    if let Ok(text) = fs::read_to_string(state_path(root)) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            match line.split_once('\t') {
                Some((name, mtime_str)) => {
                    state.insert(name.to_string(), parse_mtime_secs(mtime_str));
                }
                None => {
                    state.insert(line.to_string(), None);
                }
            }
        }
    }
    state
}

pub fn save_done(root: &Path, done: &DoneState) -> Result<()> {
    let mut text =
        String::from("# latexlings progress — safe to delete if you want to start over\n");
    for (name, mtime) in &done.entries {
        match mtime.and_then(format_mtime_secs) {
            Some(secs) => text.push_str(&format!("{name}\t{secs}\n")),
            None => text.push_str(&format!("{name}\n")),
        }
    }
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
    println!(
        "initialized latexlings practice directory: {}",
        target.display()
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn done_state_round_trips_with_and_without_mtime() {
        let dir = std::env::temp_dir().join(format!("latexlings-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut done = DoneState::default();
        let mtime = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        done.insert("intro1".to_string(), Some(mtime));
        done.insert("intro2".to_string(), None);
        save_done(&dir, &done).unwrap();

        let loaded = load_done(&dir);
        assert!(loaded.contains("intro1"));
        assert_eq!(loaded.mtime("intro1"), Some(mtime));
        assert!(loaded.contains("intro2"));
        assert_eq!(loaded.mtime("intro2"), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn done_state_reads_legacy_no_mtime_format() {
        let dir =
            std::env::temp_dir().join(format!("latexlings-test-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".latexlings-state.txt"),
            "# latexlings progress\nintro1\nintro2\n",
        )
        .unwrap();

        let loaded = load_done(&dir);
        assert!(loaded.contains("intro1"));
        assert_eq!(loaded.mtime("intro1"), None);
        assert!(loaded.contains("intro2"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
