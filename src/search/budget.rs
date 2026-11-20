//! One cooperative budget across all serial search phases of a limited move.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::SearchLimits;

const TIME_CHECK_MASK: usize = 0x3F;

#[derive(Clone, Debug, Default)]
pub(crate) struct MoveBudget(Option<Arc<Limits>>);

#[derive(Debug)]
struct Limits {
    deadline: Option<Instant>,
    max_nodes: Option<usize>,
    nodes: AtomicUsize,
    interrupted: AtomicBool,
}

impl MoveBudget {
    pub fn new(limits: SearchLimits) -> Self {
        if limits.time_limit_ms.is_none() && limits.node_limit.is_none() {
            return Self::default();
        }
        let start = Instant::now();
        let deadline = limits.time_limit_ms.map(|ms| {
            Duration::try_from_secs_f64(ms / 1000.0)
                .ok()
                .and_then(|duration| start.checked_add(duration))
                .unwrap_or(start)
        });
        Self(Some(Arc::new(Limits {
            deadline,
            max_nodes: limits.node_limit,
            nodes: AtomicUsize::new(0),
            interrupted: AtomicBool::new(false),
        })))
    }

    pub fn stopped(&self) -> bool {
        self.0.as_ref().is_some_and(|b| {
            if b.interrupted.load(Ordering::Relaxed) {
                return true;
            }
            if b.deadline.is_some_and(|d| Instant::now() >= d) {
                b.interrupted.store(true, Ordering::Relaxed);
                return true;
            }
            false
        })
    }

    pub fn enter(&self) -> bool {
        let Some(b) = &self.0 else {
            return true;
        };
        if b.interrupted.load(Ordering::Relaxed) {
            return false;
        }
        // Limited searches run serially, including all tactical subsearches.
        let count = b.nodes.load(Ordering::Relaxed);
        if b.max_nodes.is_some_and(|max| count >= max) {
            b.interrupted.store(true, Ordering::Relaxed);
            return false;
        }
        if b.deadline
            .is_some_and(|d| (count == 0 || (count & TIME_CHECK_MASK) == 0) && Instant::now() >= d)
        {
            b.interrupted.store(true, Ordering::Relaxed);
            return false;
        }
        b.nodes.store(count + 1, Ordering::Relaxed);
        true
    }

    pub fn nodes(&self) -> Option<usize> {
        self.0.as_ref().map(|b| b.nodes.load(Ordering::Relaxed))
    }

    pub fn add_nodes(&self, extra: usize) {
        if let Some(b) = &self.0 {
            b.nodes.fetch_add(extra, Ordering::Relaxed);
        }
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.0.as_ref().and_then(|b| b.deadline)
    }

    /// Creates a child budget for a tactical subsearch (such as root VCT) under
    /// a time-only move limit, capping it to `fraction` of the remaining time so
    /// alpha-beta always keeps headroom to complete a base search.
    pub fn timed_sub_budget(&self, fraction: f64) -> Option<Self> {
        let parent = self.0.as_ref()?;
        if parent.max_nodes.is_some() {
            return None;
        }
        let parent_deadline = parent.deadline?;
        let now = Instant::now();
        let remaining = parent_deadline.checked_duration_since(now)?;
        let capped = remaining.mul_f64(fraction.clamp(0.0, 1.0));
        let sub_deadline = now
            .checked_add(capped)
            .map_or(parent_deadline, |deadline| deadline.min(parent_deadline));
        Some(Self(Some(Arc::new(Limits {
            deadline: Some(sub_deadline),
            max_nodes: None,
            nodes: AtomicUsize::new(0),
            interrupted: AtomicBool::new(false),
        }))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_budget_enforces_exact_node_limit() {
        let budget = MoveBudget::new(SearchLimits {
            node_limit: Some(3),
            ..SearchLimits::default()
        });
        assert!(budget.enter());
        assert!(budget.enter());
        assert!(budget.enter());
        assert!(!budget.enter());
        assert!(budget.stopped());
        assert_eq!(budget.nodes(), Some(3));
    }

    #[test]
    fn move_budget_zero_ms_deadline_stops_on_first_enter() {
        let budget = MoveBudget::new(SearchLimits {
            time_limit_ms: Some(0.0),
            ..SearchLimits::default()
        });
        assert!(!budget.enter());
        assert!(budget.stopped());
        assert_eq!(budget.nodes(), Some(0));
    }
}
