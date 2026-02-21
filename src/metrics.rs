use std::sync::atomic::{AtomicU64, Ordering};
use serde::Serialize;

#[derive(Debug, Default)]
pub struct Metrics {
    pub exact_hits: AtomicU64,
    pub semantic_hits: AtomicU64,
    pub misses: AtomicU64,
    pub total_requests: AtomicU64,
    pub tokens_saved: AtomicU64, 
    pub tokens_used: AtomicU64,   
}

impl Metrics {
    pub fn new() -> Self {

        Self::default()

    }

    pub fn record_exact_hit(&self) {

        self.exact_hits.fetch_add(1, Ordering::Relaxed);
        self.total_requests.fetch_add(1, Ordering::Relaxed);

    }

    pub fn record_semantic_hit(&self, tokens_saved: u64) {

        self.semantic_hits.fetch_add(1, Ordering::Relaxed);
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.tokens_saved.fetch_add(tokens_saved, Ordering::Relaxed);

    }

    pub fn record_miss(&self, tokens_used: u64) {

        self.misses.fetch_add(1, Ordering::Relaxed);
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.tokens_used.fetch_add(tokens_used, Ordering::Relaxed);

    }

    pub fn snapshot(&self) -> MetricsSnapshot {

        MetricsSnapshot {
            exact_hits: self.exact_hits.load(Ordering::Relaxed),
            semantic_hits: self.semantic_hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            tokens_saved: self.tokens_saved.load(Ordering::Relaxed),
            tokens_used: self.tokens_used.load(Ordering::Relaxed),

        }
    }
}

#[derive(Debug, Serialize)]
pub struct MetricsSnapshot {
    pub exact_hits: u64,
    pub semantic_hits: u64,
    pub misses: u64,
    pub total_requests: u64,
    pub tokens_saved: u64,
    pub tokens_used: u64,
}

impl MetricsSnapshot {
    pub fn cache_hit_rate(&self) -> f64 {

        if self.total_requests == 0 {
            return 0.0;
        }
        let total_hits = self.exact_hits + self.semantic_hits;
        (total_hits as f64 / self.total_requests as f64) * 100.0

    }

    pub fn cost_saved_usd(&self) -> f64 {

        // Groq pricing: roughly $0.001 per 1K tokens (average)
        (self.tokens_saved as f64 / 1000.0) * 0.001
        
    }

    pub fn cost_spent_usd(&self) -> f64 {
        (self.tokens_used as f64 / 1000.0) * 0.001
    }
}