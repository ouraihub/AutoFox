//! Download task registry and lifecycle controls.

#[cfg(test)]
#[path = "queue_tests.rs"]
mod queue_tests;

use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ipc::{ErrorCode, IpcError};

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
    cancel: Arc<AtomicBool>,
}

/// Downloads allowed to run at once. The queue reports `Busy` beyond it.
const MAX_ACTIVE_TASKS: usize = 4;

/// Process-wide task registry managed by Tauri.
#[derive(Default)]
pub struct TaskRegistry {
    records: Mutex<BTreeMap<String, TaskRecord>>,
}

impl TaskRegistry {
    pub fn persist<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        use tauri::Manager;
        let Ok(dir) = app.path().app_config_dir() else {
            return;
        };
        if fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("tasks.json");
        let Ok(text) = serde_json::to_string_pretty(&self.list()) else {
            return;
        };
        let _ = fs::write(path, text);
    }

    pub fn load<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        use tauri::Manager;
        let Ok(dir) = app.path().app_config_dir() else {
            return;
        };
        let Ok(text) = fs::read_to_string(dir.join("tasks.json")) else {
            return;
        };
        let Ok(summaries) = serde_json::from_str::<Vec<TaskSummary>>(&text) else {
            return;
        };
        let Ok(mut records) = self.records.lock() else {
            return;
        };
        if !records.is_empty() {
            return;
        }
        for summary in summaries {
            records.insert(
                summary.task_id.clone(),
                TaskRecord {
                    summary,
                    cancel: Arc::new(AtomicBool::new(false)),
                },
            );
        }
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
    ) -> Result<(String, Arc<AtomicBool>), IpcError> {
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
    fn register(&self, kind: &str) -> Result<(String, Arc<AtomicBool>), IpcError> {
        let task_id = Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
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

    /// Returns whether a task was cancelled.
    pub fn is_cancelled(&self, task_id: &str) -> bool {
        self.records
            .lock()
            .ok()
            .and_then(|records| {
                records
                    .get(task_id)
                    .map(|record| record.cancel.load(Ordering::Relaxed))
            })
            .unwrap_or(true)
    }

    /// Marks a task terminal.
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
        record.cancel.store(true, Ordering::Relaxed);
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
        record.cancel.store(false, Ordering::Relaxed);
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
