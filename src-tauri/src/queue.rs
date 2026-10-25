//! Download task registry and lifecycle controls.

#[cfg(test)]
#[path = "queue_tests.rs"]
mod queue_tests;

use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ipc::{ErrorCode, IpcError};
use crate::logs;

const TARGET: &str = "queue";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskState {
    /// Task is running.
    Downloading,
    /// Task stopped while retaining its `.part` file.
    Paused,
    /// Task completed.
    Done,
    /// Task failed.
    Failed,
    /// Task was cancelled.
    Cancelled,
}
/// Serializable task snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    /// Stable task identifier.
    pub task_id: String,
    /// Task kind.
    pub kind: String,
    /// Current lifecycle state.
    pub state: TaskState,
    /// Output path once known.
    pub output_path: Option<String>,
    /// Error message key when failed.
    pub error_key: Option<String>,
}

struct TaskRecord {
    summary: TaskSummary,
    cancel: Arc<TaskSignal>,
}

/// Run/stop/publish arbitration shared by a task's worker and the registry.
///
/// One atomic makes "the user stopped it" and "the worker is publishing its
/// output" mutually exclusive: whichever transition lands first wins and the
/// other side sees it. A plain cancel flag left a window between the worker's
/// last check and its rename in which both happened, publishing a file for a
/// task the registry reported as cancelled.
#[derive(Debug, Default)]
pub struct TaskSignal(AtomicU8);

const RUNNING: u8 = 0;
const STOPPED: u8 = 1;
const PUBLISHING: u8 = 2;

impl TaskSignal {
    /// Whether the user cancelled or paused the task.
    pub fn is_stopped(&self) -> bool {
        self.0.load(Ordering::Acquire) == STOPPED
    }

    /// Requests a stop. Returns `false` once the worker has begun publishing.
    pub fn stop(&self) -> bool {
        self.0
            .compare_exchange(RUNNING, STOPPED, Ordering::AcqRel, Ordering::Acquire)
            .map_or_else(|current| current == STOPPED, |_| true)
    }

    /// Claims the right to publish the output. Returns `false` if a stop won.
    pub fn claim_publish(&self) -> bool {
        self.0
            .compare_exchange(RUNNING, PUBLISHING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn resume(&self) {
        self.0.store(RUNNING, Ordering::Release);
    }
}

/// Downloads allowed to run at once. The queue reports `Busy` beyond it.
const MAX_ACTIVE_TASKS: usize = 4;

/// Process-wide task registry managed by Tauri.
#[derive(Default)]
pub struct TaskRegistry {
    records: Mutex<BTreeMap<String, TaskRecord>>,
}

impl TaskRegistry {
    /// Writes the task snapshot list to `tasks.json`.
    ///
    /// Persistence is best-effort: a failure must not abort the download that
    /// triggered it. Every failure path is logged, because a silent one leaves
    /// the queue looking healthy while `tasks.json` silently goes stale.
    pub fn persist<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        use tauri::Manager;
        let Ok(dir) = app.path().app_config_dir() else {
            logs::warn(TARGET, "cannot persist tasks: no app config directory");
            return;
        };
        if let Err(error) = fs::create_dir_all(&dir) {
            logs::warn(
                TARGET,
                format!("cannot persist tasks: config directory unusable: {error}"),
            );
            return;
        }
        let path = dir.join("tasks.json");
        let text = match serde_json::to_string_pretty(&self.list()) {
            Ok(text) => text,
            Err(error) => {
                logs::error(TARGET, format!("cannot serialize task snapshots: {error}"));
                return;
            }
        };
        if let Err(error) = fs::write(path, text) {
            logs::warn(TARGET, format!("cannot write tasks.json: {error}"));
        }
    }

    /// Restores task snapshots from `tasks.json` at startup.
    ///
    /// A missing file is the normal first-run state and stays silent. Every
    /// other failure is logged: losing the queue without a trace looks
    /// identical to "the user never downloaded anything".
    pub fn load<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        use tauri::Manager;
        let Ok(dir) = app.path().app_config_dir() else {
            logs::warn(TARGET, "cannot restore tasks: no app config directory");
            return;
        };
        let text = match fs::read_to_string(dir.join("tasks.json")) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                logs::warn(TARGET, format!("cannot read tasks.json: {error}"));
                return;
            }
        };
        let summaries = match serde_json::from_str::<Vec<TaskSummary>>(&text) {
            Ok(summaries) => summaries,
            Err(error) => {
                logs::error(TARGET, format!("tasks.json is not parseable: {error}"));
                return;
            }
        };
        let Ok(mut records) = self.records.lock() else {
            logs::error(TARGET, "cannot restore tasks: registry lock poisoned");
            return;
        };
        if !records.is_empty() {
            return;
        }
        let restored = summaries.len();
        for summary in summaries {
            records.insert(
                summary.task_id.clone(),
                TaskRecord {
                    summary,
                    cancel: Arc::new(TaskSignal::default()),
                },
            );
        }
        drop(records);
        logs::info(TARGET, format!("restored {restored} task snapshot(s)"));
    }

    /// Registers a running task, enforcing the four-task process limit.
    ///
    /// Persisting is deliberately *not* done inside [`Self::register`]: the
    /// only writer of `tasks.json` is [`Self::persist`], which re-locks
    /// `records` through [`Self::list`]. `Mutex` is not reentrant, so reaching
    /// `persist` while the guard is alive blocks the caller forever with no
    /// error and no log line. Keeping the lock inside `register` and the file
    /// write out here makes that shape unrepresentable.
    pub fn start<R: tauri::Runtime>(
        &self,
        kind: &str,
        app: Option<&tauri::AppHandle<R>>,
    ) -> Result<(String, Arc<TaskSignal>), IpcError> {
        let started = self.register(kind)?;
        if let Some(app) = app {
            self.persist(app);
        }
        Ok(started)
    }

    /// Inserts a `Downloading` record, or reports `Busy` at the task limit.
    ///
    /// Holds the registry lock for its whole body and touches no other state,
    /// so it cannot reach a second `lock` call.
    fn register(&self, kind: &str) -> Result<(String, Arc<TaskSignal>), IpcError> {
        let task_id = Uuid::new_v4().to_string();
        let cancel = Arc::new(TaskSignal::default());
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal("task registry lock poisoned"))?;
        if records
            .values()
            .filter(|record| record.summary.state == TaskState::Downloading)
            .count()
            >= MAX_ACTIVE_TASKS
        {
            return Err(IpcError {
                code: ErrorCode::Busy,
                message_key: "errors.taskLimit",
                field: None,
                retryable: true,
                detail: "maximum active task count reached".to_owned(),
            });
        }
        records.insert(
            task_id.clone(),
            TaskRecord {
                summary: TaskSummary {
                    task_id: task_id.clone(),
                    kind: kind.to_owned(),
                    state: TaskState::Downloading,
                    output_path: None,
                    error_key: None,
                },
                cancel: cancel.clone(),
            },
        );
        Ok((task_id, cancel))
    }

    /// Returns whether a task was cancelled, or is no longer worth running.
    ///
    /// Called once per downloaded chunk, so only the abnormal poisoned-lock
    /// branch logs; a missing record is the ordinary "task was removed" case.
    /// Both unreadable states answer `true`: stopping a download that no
    /// longer has an owner is recoverable, writing bytes forever is not.
    pub fn is_cancelled(&self, task_id: &str) -> bool {
        let Ok(records) = self.records.lock() else {
            logs::error(
                TARGET,
                format!("registry lock poisoned; stopping task {task_id}"),
            );
            return true;
        };
        records
            .get(task_id)
            .is_none_or(|record| record.cancel.is_stopped())
    }

    /// Marks a task terminal, but never overwrites a state the user chose.
    ///
    /// The registry is the single authority on lifecycle transitions. A worker
    /// that is already cancelled or paused keeps running until its next
    /// cancellation check, so its late `finish(Done)` would otherwise flip the
    /// UI from "已取消" back to "已完成" and lose the user's decision. Only a
    /// task still in `Downloading` may be moved to a terminal state.
    pub fn finish(
        &self,
        task_id: &str,
        state: TaskState,
        output_path: Option<String>,
        error_key: Option<String>,
    ) {
        if let Ok(mut records) = self.records.lock()
            && let Some(record) = records.get_mut(task_id)
        {
            if record.summary.state != TaskState::Downloading {
                logs::info(
                    TARGET,
                    format!(
                        "ignoring late {state:?} for task {task_id} already in {:?}",
                        record.summary.state
                    ),
                );
                return;
            }
            record.summary.state = state;
            record.summary.output_path = output_path;
            record.summary.error_key = error_key;
        }
    }

    /// Cancels a task and preserves its partial file.
    pub fn cancel(&self, task_id: &str, paused: bool) -> Result<(), IpcError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal("task registry lock poisoned"))?;
        let record = records.get_mut(task_id).ok_or_else(not_found)?;
        // The worker already claimed publication: the file is being renamed
        // into place and the task is about to be `Done`, so a stop is too late.
        if !record.cancel.stop() {
            return Err(IpcError {
                code: ErrorCode::Busy,
                message_key: "errors.taskFinishing",
                field: None,
                retryable: false,
                detail: "task is already saving its output".to_owned(),
            });
        }
        record.summary.state = if paused {
            TaskState::Paused
        } else {
            TaskState::Cancelled
        };
        Ok(())
    }

    /// Returns all task snapshots.
    pub fn list(&self) -> Vec<TaskSummary> {
        self.records
            .lock()
            .map(|records| {
                records
                    .values()
                    .map(|record| record.summary.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Removes a task snapshot. Files are intentionally not deleted by this command.
    pub fn remove(&self, task_id: &str) -> Result<(), IpcError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal("task registry lock poisoned"))?;
        records.remove(task_id).map(|_| ()).ok_or_else(not_found)
    }

    /// Moves a paused task back to a resumable queued state.
    pub fn resume(&self, task_id: &str) -> Result<(), IpcError> {
        let mut records = self
            .records
            .lock()
            .map_err(|_| internal("task registry lock poisoned"))?;
        let record = records.get_mut(task_id).ok_or_else(not_found)?;
        if record.summary.state != TaskState::Paused {
            return Err(IpcError {
                code: ErrorCode::Validation,
                message_key: "errors.taskNotPaused",
                field: Some("taskId"),
                retryable: false,
                detail: "task is not paused".to_owned(),
            });
        }
        record.cancel.resume();
        record.summary.state = TaskState::Downloading;
        Ok(())
    }
}

/// Lists task snapshots.
#[tauri::command]
pub fn list_tasks(state: tauri::State<'_, TaskRegistry>) -> Vec<TaskSummary> {
    state.list()
}

/// Cancels a running task.
#[tauri::command]
pub fn cancel_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskRegistry>,
    task_id: String,
) -> Result<(), IpcError> {
    let result = state.cancel(&task_id, false);
    state.persist(&app);
    result
}

/// Pauses a task by cancelling the current HTTP stream and retaining its `.part` file.
#[tauri::command]
pub fn pause_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskRegistry>,
    task_id: String,
) -> Result<(), IpcError> {
    let result = state.cancel(&task_id, true);
    state.persist(&app);
    result
}

/// Resumes a paused task.
#[tauri::command]
pub fn resume_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskRegistry>,
    task_id: String,
) -> Result<(), IpcError> {
    let result = state.resume(&task_id);
    state.persist(&app);
    result
}

/// Removes a task snapshot. Files are intentionally not deleted by this command.
#[tauri::command]
pub fn remove_task(
    app: tauri::AppHandle,
    state: tauri::State<'_, TaskRegistry>,
    task_id: String,
) -> Result<(), IpcError> {
    let result = state.remove(&task_id);
    state.persist(&app);
    result
}

fn not_found() -> IpcError {
    IpcError {
        code: ErrorCode::NotFound,
        message_key: "errors.taskNotFound",
        field: Some("taskId"),
        retryable: false,
        detail: "task was not found".to_owned(),
    }
}
fn internal(detail: &'static str) -> IpcError {
    IpcError {
        code: ErrorCode::Internal,
        message_key: "errors.internal",
        field: None,
        retryable: false,
        detail: detail.to_owned(),
    }
}
