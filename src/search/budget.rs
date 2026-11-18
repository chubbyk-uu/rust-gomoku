//! One cooperative budget across all serial search phases of a limited move.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use super::SearchLimits;

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
            b.interrupted.load(Ordering::Relaxed) || b.deadline.is_some_and(|d| Instant::now() >= d)
        })
    }

    pub fn enter(&self) -> bool {
        let Some(b) = &self.0 else {
            return true;
        };
        if self.stopped() {
            return false;
        }
        // Limited searches run serially, including all tactical subsearches.
        let count = b.nodes.load(Ordering::Relaxed);
        if b.max_nodes.is_some_and(|max| count >= max) {
            b.interrupted.store(true, Ordering::Relaxed);
            return false;
        }
        b.nodes.store(count + 1, Ordering::Relaxed);
        true
    }

    pub fn nodes(&self) -> Option<usize> {
        self.0.as_ref().map(|b| b.nodes.load(Ordering::Relaxed))
    }
}
