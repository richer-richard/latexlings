//! Shared parallel verification engine used by the TUI "check all" mode,
//! `latexlings verify`, and `dev-check`'s solution-verification pass.

// This module's whole public API has no caller yet — Tasks 3-6 wire it into
// the CLI `verify` command, `dev-check`, and a new TUI mode. Allow dead_code
// module-wide rather than repeating the annotation on every item until then.
#![allow(dead_code)]

use crate::info::Exercise;
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
            if tx.send(JobResult { index: job.index, status }).is_err() {
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
        }
    }

    #[test]
    fn run_blocking_returns_one_result_per_job_tagged_by_index() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = Arc::clone(&calls);
        let verifier: Verifier = Arc::new(move |_root, ex| {
            calls_clone.fetch_add(1, Ordering::SeqCst);
            if ex.name == "fails" { Status::CompileFail("boom".into()) } else { Status::Done }
        });

        let jobs = vec![
            Job { index: 0, root: PathBuf::from("/tmp"), exercise: fake_exercise("a") },
            Job { index: 1, root: PathBuf::from("/tmp"), exercise: fake_exercise("fails") },
            Job { index: 2, root: PathBuf::from("/tmp"), exercise: fake_exercise("c") },
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
}
