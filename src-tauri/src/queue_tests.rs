//! Lock-discipline and concurrency-limit tests for [`TaskRegistry`].

use super::{MAX_ACTIVE_TASKS, TaskRegistry, TaskState};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// How long a lock-free operation may take before the test calls it deadlocked.
/// Generous: the guarded work is a `BTreeMap` insert on an in-memory map.
const DEADLINE: Duration = Duration::from_secs(10);

/// Runs `body` on its own thread and fails if it does not return in time.
///
/// A deadlock never returns, so asserting on the result directly would hang the
/// whole suite instead of reporting a failure. The worker thread stays parked on
/// the lock in that case; the process is about to exit anyway.
fn with_deadline<T: Send + 'static>(label: &str, body: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(body());
    });
    rx.recv_timeout(DEADLINE).unwrap_or_else(|_| {
        panic!("{label} did not finish within {DEADLINE:?}: lock held too long")
    })
}

#[test]
fn register_releases_the_lock_before_returning() {
    // `start` calls `register` and then `persist`, and `persist` re-locks
    // `records` through `list`. `Mutex` is not reentrant, so a guard still
    // alive after `register` returns would hang the download command with no
    // error and no log line. Taking the lock again here proves it is free.
    let outcome = with_deadline("register then list", || {
        let registry = TaskRegistry::default();
        let (task_id, _cancel) = registry
            .register("direct")
            .unwrap_or_else(|e| panic!("register should insert a task: {e:?}"));
        // This is the exact call `persist` makes. Under the old code it was
        // reached with the guard held.
        let listed = registry.list();
        (task_id, listed)
    });
    let (task_id, listed) = outcome;
    assert_eq!(listed.len(), 1, "the registered task should be listed");
    assert_eq!(listed[0].task_id, task_id);
}

#[test]
fn register_marks_the_task_downloading() {
    let registry = TaskRegistry::default();
    let (task_id, cancel) = registry
        .register("direct")
        .unwrap_or_else(|e| panic!("register should insert a task: {e:?}"));
    let listed = registry.list();
    assert_eq!(listed.len(), 1, "exactly one task should be listed");
    assert_eq!(listed[0].task_id, task_id);
    assert_eq!(listed[0].kind, "direct");
    assert_eq!(listed[0].state, TaskState::Downloading);
    assert!(listed[0].output_path.is_none(), "no output path yet");
    assert!(
        !cancel.load(Ordering::Relaxed),
        "a fresh task is not cancelled"
    );
}

#[test]
fn register_rejects_one_task_past_the_limit() {
    let registry = TaskRegistry::default();
    for i in 0..MAX_ACTIVE_TASKS {
        registry
            .register("direct")
            .unwrap_or_else(|e| panic!("task {i} should register: {e:?}"));
    }
    let error = registry
        .register("direct")
        .expect_err("one task past the limit should be rejected");
    assert_eq!(error.message_key, "errors.taskLimit");
    assert!(error.retryable, "a busy queue is retryable");
    assert_eq!(
        registry.list().len(),
        MAX_ACTIVE_TASKS,
        "the rejected task must not be stored"
    );
}

#[test]
fn a_terminal_task_frees_a_slot() {
    let registry = TaskRegistry::default();
    let mut ids = Vec::new();
    for i in 0..MAX_ACTIVE_TASKS {
        let (id, _) = registry
            .register("direct")
            .unwrap_or_else(|e| panic!("task {i} should register: {e:?}"));
        ids.push(id);
    }
    let finished = ids.first().expect("tasks were registered").clone();
    registry.finish(
        &finished,
        TaskState::Done,
        Some("D:\\out\\clip.mp4".to_owned()),
        None,
    );
    registry
        .register("direct")
        .expect("a finished task should free a slot");
    let done = registry
        .list()
        .into_iter()
        .find(|s| s.task_id == finished)
        .expect("the finished task stays listed");
    assert_eq!(done.state, TaskState::Done);
    assert_eq!(done.output_path.as_deref(), Some("D:\\out\\clip.mp4"));
}

#[test]
fn a_paused_task_does_not_hold_a_slot_open_forever() {
    // Pausing leaves the record in place but out of the `Downloading` count,
    // so the freed slot is usable. This is the invariant P1.1 scheduling needs.
    let registry = TaskRegistry::default();
    let mut ids = Vec::new();
    for i in 0..MAX_ACTIVE_TASKS {
        let (id, _) = registry
            .register("direct")
            .unwrap_or_else(|e| panic!("task {i} should register: {e:?}"));
        ids.push(id);
    }
    let paused = ids.first().expect("tasks were registered").clone();
    registry
        .cancel(&paused, true)
        .expect("an active task can be paused");
    registry
        .register("direct")
        .expect("a paused task should not occupy an active slot");
    let state = registry
        .list()
        .into_iter()
        .find(|s| s.task_id == paused)
        .expect("the paused task stays listed")
        .state;
    assert_eq!(state, TaskState::Paused);
}
