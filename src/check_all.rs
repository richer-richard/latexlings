//! Shared parallel verification engine used by the TUI "check all" mode,
//! `latexlings verify`, and `dev-check`'s solution-verification pass.

use crate::info::{self, DoneState, Exercise};
use crate::verify::Status;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;

/// One unit of parallel verification work: an exercise checked against a
/// specific root (the real practice root for normal checks, or a
/// per-exercise scratch root for dev-check's solution pass).
pub struct Job {
    pub index: usize,
    pub root: PathBuf,
    pub exercise: Exercise,
}

/// A job's outcome, tagged with its original index so callers can map
/// results back to input order regardless of completion order.
pub struct JobResult {
    pub index: usize,
    pub status: Status,
}

/// Injectable verification function so this engine is testable without a
/// real pdflatex invocation. Production callers pass `Arc::new(crate::verify::verify)`.
pub type Verifier = Arc<dyn Fn(&Path, &Exercise) -> Status + Send + Sync>;

/// Spawn `available_parallelism()` worker threads pulling from `jobs` via a
/// shared atomic cursor. Returns immediately with a `Receiver` the caller
/// drains as results land, in completion order (not input order) — use
/// `JobResult::index` to recover input order.
pub fn spawn(jobs: Vec<Job>, verifier: Verifier) -> Receiver<JobResult> {
    let (tx, rx) = mpsc::channel();
    let n_jobs = jobs.len();
    let jobs = Arc::new(jobs);
    let cursor = Arc::new(AtomicUsize::new(0));
    let n_workers = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(n_jobs.max(1));

    for _ in 0..n_workers {
        let jobs = Arc::clone(&jobs);
        let cursor = Arc::clone(&cursor);
        let tx = tx.clone();
        let verifier = Arc::clone(&verifier);
        thread::spawn(move || loop {
            let i = cursor.fetch_add(1, Ordering::Relaxed);
            let Some(job) = jobs.get(i) else { break };
            let status = verifier(&job.root, &job.exercise);
            if tx
                .send(JobResult {
                    index: job.index,
                    status,
                })
                .is_err()
            {
                break;
            }
        });
    }
    rx
}

/// Blocking convenience wrapper: run all jobs and collect every result.
pub fn run_blocking(jobs: Vec<Job>, verifier: Verifier) -> Vec<JobResult> {
    let n = jobs.len();
    if n == 0 {
        return Vec::new();
    }
    let rx = spawn(jobs, verifier);
    let mut results = Vec::with_capacity(n);
    for _ in 0..n {
        match rx.recv() {
            Ok(r) => results.push(r),
            Err(_) => break,
        }
    }
    results
}

/// Build the job list for a full sweep against the real practice root,
/// skipping exercises whose `.tex` mtime is unchanged since they were last
/// verified Done — those are reported immediately as cached `Status::Done`
/// without spawning pdflatex.
pub fn plan_sweep(
    root: &Path,
    exercises: &[Exercise],
    done: &DoneState,
) -> (Vec<Job>, Vec<(usize, Status)>) {
    let mut jobs = Vec::new();
    let mut cached = Vec::new();
    for (i, ex) in exercises.iter().enumerate() {
        let current_mtime = std::fs::metadata(ex.path(root))
            .and_then(|m| m.modified())
            .ok()
            .map(info::truncate_to_secs);
        let unchanged = done.contains(&ex.name)
            && current_mtime.is_some()
            && done.mtime(&ex.name) == current_mtime;
        if unchanged {
            cached.push((i, Status::Done));
        } else {
            jobs.push(Job {
                index: i,
                root: root.to_path_buf(),
                exercise: ex.clone(),
            });
        }
    }
    (jobs, cached)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::info::Exercise;
    use crate::info::Mode;
    use crate::verify::Status;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn fake_exercise(name: &str) -> Exercise {
        Exercise {
            name: name.to_string(),
            dir: "00_intro".to_string(),
            mode: Mode::Fix,
            hint: "hint".to_string(),
            checks: vec![],
            compiles_as_shipped: false,
            strict_chktex: false,
        }
    }

    #[test]
    fn run_blocking_returns_one_result_per_job_tagged_by_index() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let verifier: Verifier = Arc::new(move |_root, ex| {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            if ex.name == "fails" {
                Status::CompileFail("boom".into())
            } else {
                Status::Done
            }
        });

        let jobs = vec![
            Job {
                index: 0,
                root: PathBuf::from("/tmp"),
                exercise: fake_exercise("a"),
            },
            Job {
                index: 1,
                root: PathBuf::from("/tmp"),
                exercise: fake_exercise("fails"),
            },
            Job {
                index: 2,
                root: PathBuf::from("/tmp"),
                exercise: fake_exercise("c"),
            },
        ];

        let mut results = run_blocking(jobs, verifier);
        results.sort_by_key(|r| r.index);

        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(results.len(), 3);
        assert!(results[0].status.is_done());
        assert!(!results[1].status.is_done());
        assert!(results[2].status.is_done());
    }

    #[test]
    fn run_blocking_on_empty_jobs_returns_empty() {
        let verifier: Verifier = Arc::new(|_, _| Status::Done);
        let results = run_blocking(Vec::new(), verifier);
        assert!(results.is_empty());
    }

    use crate::info::DoneState;
    use std::time::{Duration, SystemTime};

    #[test]
    fn plan_sweep_skips_unchanged_done_exercises_and_queues_the_rest() {
        let dir = std::env::temp_dir().join(format!("latexlings-plan-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("exercises/00_intro")).unwrap();
        let file_a = dir.join("exercises/00_intro/a.tex");
        let file_b = dir.join("exercises/00_intro/b.tex");
        std::fs::write(&file_a, "unchanged").unwrap();
        std::fs::write(&file_b, "will be marked stale").unwrap();

        let exercises = vec![fake_exercise("a"), fake_exercise("b"), fake_exercise("c")];
        let a_mtime = std::fs::metadata(&file_a).unwrap().modified().unwrap();

        let mut done = DoneState::default();
        // Real `DoneState` entries are always whole-second (every insertion
        // site truncates before storing, matching what a disk round trip
        // through `.latexlings-state.txt` would produce), so mirror that here
        // rather than storing a raw, full-precision mtime.
        done.insert("a".to_string(), Some(info::truncate_to_secs(a_mtime)));
        // "b" is done but with a stale mtime far in the past -> must be rechecked.
        done.insert(
            "b".to_string(),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
        );
        // "c" was never verified -> must be checked.

        let (jobs, cached) = plan_sweep(&dir, &exercises, &done);

        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].0, 0); // index of "a"
        assert!(cached[0].1.is_done());

        let mut job_names: Vec<&str> = jobs.iter().map(|j| j.exercise.name.as_str()).collect();
        job_names.sort();
        assert_eq!(job_names, vec!["b", "c"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn plan_sweep_hits_cache_after_mtime_round_trips_through_saved_state() {
        // Regression test for the mtime-precision mismatch: `.latexlings-state.txt`
        // persists mtimes truncated to whole-second resolution (see
        // `info::format_mtime_secs`/`parse_mtime_secs`), but a fresh
        // `std::fs::metadata(...).modified()` read is full (sub-second)
        // precision on filesystems like APFS/ext4. Every real
        // `latexlings verify` invocation reloads `DoneState` from disk (already
        // truncated) in a fresh process, then compares it against a fresh,
        // untruncated mtime read. If `plan_sweep` used exact `SystemTime`
        // equality across that precision mismatch, the cache hit would be
        // defeated on nearly every real invocation — this test builds the
        // `DoneState` via an actual save/load round trip (not purely in
        // memory) to reproduce that exact scenario.
        let dir =
            std::env::temp_dir().join(format!("latexlings-plan-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("exercises/00_intro")).unwrap();
        let file_a = dir.join("exercises/00_intro/a.tex");
        std::fs::write(&file_a, "unchanged").unwrap();

        let exercises = vec![fake_exercise("a")];
        let fresh_mtime = std::fs::metadata(&file_a).unwrap().modified().unwrap();

        // Simulate what `mark_done` persisted after a prior `latexlings verify`
        // run: the mtime captured (and truncated) at verification time,
        // written to `.latexlings-state.txt`, then reloaded fresh as a new
        // process would on its next invocation.
        let mut done = DoneState::default();
        done.insert("a".to_string(), Some(info::truncate_to_secs(fresh_mtime)));
        info::save_done(&dir, &done).unwrap();
        let done = info::load_done(&dir);

        // The file is untouched since verification, so a fresh metadata read
        // compared against the reloaded, disk-truncated state must still
        // count as a cache hit — not be silently reverified.
        let (jobs, cached) = plan_sweep(&dir, &exercises, &done);

        assert_eq!(
            cached.len(),
            1,
            "expected 'a' to be a cache hit after its mtime round-tripped through disk state"
        );
        assert_eq!(cached[0].0, 0);
        assert!(cached[0].1.is_done());
        assert!(jobs.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
