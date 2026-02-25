use std::collections::HashMap;
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Registry mapping task IDs to cancellation tokens.
///
/// Workers check their token to detect cancellation requests.
/// The dispatcher registers tokens before spawning tasks and
/// cleans them up after completion.
pub struct CancellationRegistry {
    tokens: Mutex<HashMap<Uuid, CancellationToken>>,
}

impl CancellationRegistry {
    pub fn new() -> Self {
        Self {
            tokens: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new cancellation token for a task. Returns the token.
    pub fn register(&self, task_id: Uuid) -> CancellationToken {
        let token = CancellationToken::new();
        self.tokens
            .lock()
            .expect("cancellation registry lock poisoned")
            .insert(task_id, token.clone());
        token
    }

    /// Signal cancellation for a task. Returns `true` if the task was found.
    pub fn cancel(&self, task_id: Uuid) -> bool {
        let guard = self
            .tokens
            .lock()
            .expect("cancellation registry lock poisoned");
        guard.get(&task_id).is_some_and(|token| {
            token.cancel();
            true
        })
    }

    /// Remove a token after the task completes (cleanup).
    pub fn remove(&self, task_id: Uuid) {
        self.tokens
            .lock()
            .expect("cancellation registry lock poisoned")
            .remove(&task_id);
    }

    /// Check if a task has been cancelled.
    pub fn is_cancelled(&self, task_id: Uuid) -> bool {
        let guard = self
            .tokens
            .lock()
            .expect("cancellation registry lock poisoned");
        guard
            .get(&task_id)
            .is_some_and(CancellationToken::is_cancelled)
    }
}

impl Default for CancellationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_cancel() {
        let registry = CancellationRegistry::new();
        let id = Uuid::new_v4();
        let token = registry.register(id);

        assert!(!registry.is_cancelled(id));
        assert!(!token.is_cancelled());

        assert!(registry.cancel(id));
        assert!(registry.is_cancelled(id));
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_unknown_returns_false() {
        let registry = CancellationRegistry::new();
        assert!(!registry.cancel(Uuid::new_v4()));
    }

    #[test]
    fn remove_cleans_up() {
        let registry = CancellationRegistry::new();
        let id = Uuid::new_v4();
        let _token = registry.register(id);

        registry.remove(id);
        assert!(!registry.is_cancelled(id));
        assert!(!registry.cancel(id));
    }
}
