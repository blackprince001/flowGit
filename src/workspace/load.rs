/// Where a request stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    /// Nothing running. The last request, if any, succeeded.
    #[default]
    Idle,
    /// A request is running. `cached` when `data` came from the disk cache meanwhile.
    Loading { cached: bool },
    /// The last request failed. `data` still holds the last good answer, if any.
    Failed(String),
}

/// A request's lifecycle. Data stays visible while a refresh is in flight,
/// and after a refresh fails.
pub struct Load<T> {
    pub data: Option<T>,
    pub status: Status,
    /// When a view last asked for this, for dropping the least recently used.
    pub used: u64,
}
impl<T> Default for Load<T> {
    fn default() -> Self {
        Self {
            data: None,
            status: Status::Idle,
            used: 0,
        }
    }
}
impl<T> Load<T> {
    /// Still waiting for the first answer.
    pub fn pending(&self) -> bool {
        self.data.is_none() && !matches!(self.status, Status::Failed(_))
    }
    pub fn loading(&self) -> bool {
        matches!(self.status, Status::Loading { .. })
    }
    pub fn error(&self) -> Option<&str> {
        match &self.status {
            Status::Failed(e) => Some(e),
            _ => None,
        }
    }
    /// `data` came from the disk cache and a live request is still running.
    /// No view reads this yet (the old `stale` flag was written and never read).
    #[allow(dead_code)]
    pub fn stale(&self) -> bool {
        matches!(self.status, Status::Loading { cached: true })
    }
    /// A request begins. Whatever data there is stays.
    pub fn start(&mut self) {
        self.status = Status::Loading { cached: false };
    }
    /// Show the disk cache's copy while the live request runs. Only
    /// applies when a request is running and there is nothing to show yet,
    /// so a slot cleared or filled meanwhile is left alone. Says whether it applied.
    pub fn show_cached(&mut self, v: T) -> bool {
        if !self.loading() || self.data.is_some() {
            return false;
        }
        self.data = Some(v);
        self.status = Status::Loading { cached: true };
        true
    }
    /// The request ended. A failure keeps the data that was already shown.
    pub fn finish(&mut self, result: Result<T, String>) {
        match result {
            Ok(v) => {
                self.data = Some(v);
                self.status = Status::Idle;
            }
            Err(e) => self.status = Status::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Load, Status};

    #[test]
    fn starts_idle_and_pending() {
        let l = Load::<u32>::default();
        assert!(l.pending());
        assert!(!l.loading());
        assert!(!l.stale());
        assert_eq!(l.error(), None);
    }

    #[test]
    fn cached_copy_then_live_answer() {
        let mut l = Load::<u32>::default();
        l.start();
        assert!(l.loading() && !l.stale() && l.pending());
        assert!(l.show_cached(1));
        assert!(l.loading() && l.stale() && !l.pending());
        assert_eq!(l.data, Some(1));
        l.finish(Ok(2));
        assert_eq!(l.data, Some(2));
        assert_eq!(l.status, Status::Idle);
        assert!(!l.loading() && !l.stale());
    }

    #[test]
    fn failure_keeps_old_data() {
        let mut l = Load::<u32>::default();
        l.start();
        l.finish(Ok(1));
        l.start();
        assert!(l.loading() && !l.stale());
        l.finish(Err("boom".into()));
        assert_eq!(l.data, Some(1));
        assert_eq!(l.error(), Some("boom"));
        assert!(!l.loading());
        assert!(!l.pending());
    }

    #[test]
    fn failure_clears_cached_flag() {
        let mut l = Load::<u32>::default();
        l.start();
        assert!(l.show_cached(1));
        l.finish(Err("offline".into()));
        assert_eq!(l.data, Some(1));
        assert!(!l.stale());
        assert_eq!(l.error(), Some("offline"));
    }

    #[test]
    fn a_failed_slot_is_not_pending_and_a_retry_clears_the_error() {
        let mut l = Load::<u32>::default();
        l.start();
        l.finish(Err("x".into()));
        assert!(!l.pending());
        l.start();
        assert_eq!(l.error(), None);
        l.finish(Ok(3));
        assert_eq!(l.error(), None);
        assert_eq!(l.data, Some(3));
    }

    #[test]
    fn cache_is_ignored_unless_loading_and_empty() {
        let mut l = Load::<u32>::default();
        // Cleared before the cache answered: nothing running.
        assert!(!l.show_cached(1));
        assert_eq!(l.data, None);
        // Filled meanwhile.
        l.start();
        l.data = Some(5);
        assert!(!l.show_cached(1));
        assert_eq!(l.data, Some(5));
        assert!(!l.stale());
    }
}
