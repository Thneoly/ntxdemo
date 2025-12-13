use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::network::Ipv4Addr;

use super::token::Token;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FlowKey {
    pub dst_ip: Ipv4Addr,
    pub dst_port: u16,
    pub src_port: u16,
    pub token: Token,
}

#[derive(Clone, Copy, Debug)]
struct Outstanding {
    sent_at: Instant,
}

#[derive(Debug, Default, Clone)]
pub struct RttStats {
    pub samples: u64,
    pub min_us: u64,
    pub max_us: u64,
    pub sum_us: u128,
}

impl RttStats {
    pub fn record(&mut self, dur: Duration) {
        let us = dur.as_micros() as u64;
        self.samples += 1;
        if self.samples == 1 {
            self.min_us = us;
            self.max_us = us;
        } else {
            self.min_us = self.min_us.min(us);
            self.max_us = self.max_us.max(us);
        }
        self.sum_us += us as u128;
    }

    pub fn avg_us(&self) -> Option<u64> {
        if self.samples == 0 {
            None
        } else {
            Some((self.sum_us / self.samples as u128) as u64)
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct MatcherStats {
    pub sent: u64,
    pub matched: u64,
    pub timeouts: u64,
    pub rtt: RttStats,
}

/// Tracks outstanding requests and matches replies.
///
/// MVP-3 design:
/// - insert() on send: (dst_ip,dst_port,src_port,token) -> sent_at
/// - on_reply() on receive: same key -> RTT
/// - timeout sweep based on a fixed timeout
pub struct Matcher {
    timeout: Duration,
    map: HashMap<FlowKey, Outstanding>,
    // FIFO queue for approximate timeout sweeping.
    order: VecDeque<(FlowKey, Instant)>,
    pub stats: MatcherStats,
}

impl Matcher {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            map: HashMap::new(),
            order: VecDeque::new(),
            stats: MatcherStats::default(),
        }
    }

    pub fn insert(&mut self, key: FlowKey) {
        let now = Instant::now();
        self.stats.sent = self.stats.sent.wrapping_add(1);
        self.map.insert(key, Outstanding { sent_at: now });
        self.order.push_back((key, now));
    }

    pub fn on_reply(&mut self, key: FlowKey) {
        if let Some(out) = self.map.remove(&key) {
            self.stats.matched = self.stats.matched.wrapping_add(1);
            self.stats.rtt.record(out.sent_at.elapsed());
        }
    }

    pub fn sweep_timeouts(&mut self) {
        let now = Instant::now();
        while let Some((key, ts)) = self.order.front().copied() {
            if now.duration_since(ts) < self.timeout {
                break;
            }
            self.order.pop_front();
            // Might have been removed already by a match.
            if self.map.remove(&key).is_some() {
                self.stats.timeouts = self.stats.timeouts.wrapping_add(1);
            }
        }
    }

    pub fn outstanding(&self) -> usize {
        self.map.len()
    }
}
