//! Background task manager — tracks concurrently running sub-agent tasks.
//!
//! When the main agent delegates in interactive mode:
//! - The sub-agent is spawned as a background `tokio::spawn` task.
//! - Its entry is registered here so the main loop knows what is running.
//! - If the **same** agent is targeted again while a task is active, the new
//!   task is queued (backlinked) instead of spawned immediately.
//! - When a task finishes, `finish()` removes it and returns the next queued
//!   task (if any) so the caller can spawn it.

use std::collections::VecDeque;

/// Result produced by a finished background agent task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    /// ID returned by [`TaskManager::register`].
    pub task_id: String,
    /// Which agent produced this result.
    pub agent_name: String,
    /// Original task description (for display).
    pub description: String,
    /// The agent's final output text.
    pub output: String,
}

/// A currently running background agent task.
#[derive(Debug)]
pub struct AgentTask {
    /// Unique ID (UUID v4 string).
    pub id: String,
    /// Name of the agent executing the task.
    pub agent_name: String,
    /// Short task description (first 120 chars of the task string).
    pub description: String,
    /// Follow-up tasks queued to run sequentially after this one finishes.
    queue: VecDeque<String>,
}

impl AgentTask {
    /// Number of tasks currently queued behind this one.
    pub fn queued_count(&self) -> usize {
        self.queue.len()
    }
}

/// Tracks all concurrently running background agent tasks.
///
/// Lives in the interactive agent loop in `cmd_agent`.
pub struct TaskManager {
    tasks: Vec<AgentTask>,
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManager {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    /// Register a newly spawned task. Returns its unique ID.
    pub fn register(&mut self, agent_name: &str, description: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let short_desc: String = description.chars().take(120).collect();
        self.tasks.push(AgentTask {
            id: id.clone(),
            agent_name: agent_name.to_string(),
            description: short_desc,
            queue: VecDeque::new(),
        });
        id
    }

    /// Find an active task for `agent_name`. Returns its ID if one is running.
    pub fn find_for_agent(&self, agent_name: &str) -> Option<&str> {
        self.tasks
            .iter()
            .find(|t| t.agent_name == agent_name)
            .map(|t| t.id.as_str())
    }

    /// Append a follow-up task to `task_id`'s queue (backlink).
    /// Returns `false` if `task_id` is not currently tracked.
    pub fn enqueue(&mut self, task_id: &str, follow_up: &str) -> bool {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.queue.push_back(follow_up.to_string());
            true
        } else {
            false
        }
    }

    /// Mark a task done — **always removes it** from the active list.
    ///
    /// Returns `Some((agent_name, next_task))` when there is a queued
    /// follow-up that should be spawned immediately, `None` otherwise.
    pub fn finish(&mut self, task_id: &str) -> Option<(String, String)> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == task_id) {
            let mut task = self.tasks.remove(pos);
            task.queue.pop_front().map(|next| (task.agent_name, next))
        } else {
            None
        }
    }

    /// All currently active tasks (for user-facing `tasks` / `status` command).
    pub fn active(&self) -> &[AgentTask] {
        &self.tasks
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_find() {
        let mut tm = TaskManager::new();
        let id = tm.register("developer", "build a REST API");
        assert_eq!(tm.find_for_agent("developer"), Some(id.as_str()));
        assert_eq!(tm.find_for_agent("creative"), None);
    }

    #[test]
    fn enqueue_and_finish_returns_next() {
        let mut tm = TaskManager::new();
        let id = tm.register("developer", "task 1");
        assert!(tm.enqueue(&id, "task 2"));
        assert!(tm.enqueue(&id, "task 3"));
        let next = tm.finish(&id);
        assert_eq!(next, Some(("developer".to_string(), "task 2".to_string())));
        // task is removed; the new id should be registered by the caller
        assert!(tm.is_empty());
    }

    #[test]
    fn finish_no_queue_returns_none() {
        let mut tm = TaskManager::new();
        let id = tm.register("developer", "task only");
        let next = tm.finish(&id);
        assert!(next.is_none());
        assert!(tm.is_empty());
    }

    #[test]
    fn finish_unknown_id_returns_none() {
        let mut tm = TaskManager::new();
        assert!(tm.finish("nonexistent").is_none());
    }
}
