/// Scheduler WASM Component Implementation
///
/// This module implements the scheduler as a WASM component
use anyhow::Result;

use crate::engine::{LoadExecutionSummary, SchedulerPipeline};

// Generate bindings for the component
wit_bindgen::generate!({
    world: "scheduler:main/scheduler-component",
    path: [
        "../wit/core",
        "../wit/eventbus",
        "../wit/protocol",
        "../wit/net",
        "../wit/scheduler",
    ],
    generate_all,
    debug: true,
});

struct SchedulerComponent;

impl Guest for SchedulerComponent {
    fn run_scenario(scenario_yaml: String) -> Result<String, String> {
        match run_scenario_impl(&scenario_yaml) {
            Ok(summary) => Ok(summary),
            Err(e) => Err(format!("Scenario execution failed: {:#}", e)),
        }
    }
}

impl exports::scheduler::net::packet::Guest for SchedulerComponent {
    fn on_udp(
        _meta: exports::scheduler::net::packet::UdpMeta,
        payload: Vec<u8>,
    ) -> Result<Option<exports::scheduler::net::packet::UdpResponse>, String> {
        // MVP-0 behavior: echo payload back.
        // Host is responsible for swapping tuple and building L2/L3/L4 headers.
        Ok(Some(exports::scheduler::net::packet::UdpResponse {
            payload,
        }))
    }
}

fn run_scenario_impl(scenario_yaml: &str) -> Result<String> {
    let mut pipeline = SchedulerPipeline::load_from_yaml_str(scenario_yaml)
        .map_err(|e| anyhow::anyhow!("Failed to load scenario: {e}"))?;

    let summary = pipeline.summary();
    let scenario_name = pipeline.scenario().name.clone();
    println!("Scenario: {}", scenario_name);
    println!("Resources: {}", summary.resources);
    println!("Metrics: {}", summary.metrics);
    println!("Tasks: {}", summary.tasks);
    println!("Edges: {}", summary.edges);

    let load_summary = pipeline
        .run_load_default()
        .map_err(|e| anyhow::anyhow!("Scenario execution failed: {e}"))?;

    Ok(format_summary(&load_summary))
}

fn format_summary(summary: &LoadExecutionSummary) -> String {
    let mut report = String::new();
    report.push_str("\n📈 Load Test Summary\n");
    report.push_str("═══════════════════════════════════════\n");
    report.push_str(&format!("Scenario: {}\n", summary.scenario_name));
    report.push_str(&format!("Total users spawned: {}\n", summary.total_users));
    report.push_str(&format!(
        "Total actions executed: {}\n",
        summary.traces.len()
    ));

    if !summary.traces.is_empty() {
        let mut durations: Vec<u64> = summary.traces.iter().map(|t| t.duration_ms).collect();
        durations.sort_unstable();
        let total_duration_ms: u64 = durations.iter().sum();
        let avg_duration = total_duration_ms as f64 / durations.len() as f64;

        let p50 = percentile(&durations, 50);
        let p95 = percentile(&durations, 95);
        let p99 = percentile(&durations, 99);

        report.push_str("\nLatency Statistics:\n");
        report.push_str(&format!("  Average: {:.2}ms\n", avg_duration));
        report.push_str(&format!("  P50: {}ms\n", p50));
        report.push_str(&format!("  P95: {}ms\n", p95));
        report.push_str(&format!("  P99: {}ms\n", p99));
        report.push_str(&format!("  Min: {}ms\n", durations[0]));
        report.push_str(&format!("  Max: {}ms\n", durations[durations.len() - 1]));
    }

    if summary.ip_binding.requested {
        report.push_str("\nIP Binding:\n");
        report.push_str(&format!("  Requested: {}\n", summary.ip_binding.requested));
        report.push_str(&format!("  Permitted: {}\n", summary.ip_binding.permitted));
        if !summary.ip_binding.pool_stats.is_empty() {
            for stat in &summary.ip_binding.pool_stats {
                report.push_str(&format!("  {}\n", stat));
            }
        }
    }

    report
}

fn percentile(durations: &[u64], pct: usize) -> u64 {
    if durations.is_empty() {
        return 0;
    }
    let len = durations.len();
    let idx = (len - 1) * pct / 100;
    durations[idx]
}

export!(SchedulerComponent);
