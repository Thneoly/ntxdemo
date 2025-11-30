/// Scheduler WASM Component Implementation
///
/// This module implements the scheduler as a WASM component
use anyhow::{Context, Result};

use crate::{IpPoolManager, UserContext, UserExecutor, parse_duration};
use scheduler_actions_http::HttpActionComponent;
use scheduler_core::dsl::Scenario;

// Generate bindings for the component
wit_bindgen::generate!({
    world: "scheduler-component",
    path: "wit",
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

fn run_scenario_impl(scenario_yaml: &str) -> Result<String> {
    // Parse scenario
    let scenario =
        Scenario::from_yaml_str(scenario_yaml).context("Failed to parse scenario YAML")?;

    scenario.validate().context("Scenario validation failed")?;

    let scenario_name = scenario.name.clone();

    // Check if load testing is enabled
    let Some(load_config) = &scenario.load else {
        return Err(anyhow::anyhow!(
            "Load configuration is required for WASM component"
        ));
    };

    println!("🚀 Running load test: {}", scenario_name);
    println!("Ramp-up phases: {}", load_config.ramp_up.phases.len());
    println!("User lifetime: {:?}", load_config.user_lifetime.mode);
    println!("Iterations: {}", load_config.user_lifetime.iterations);
    println!("Think time: {}", load_config.user_lifetime.think_time);

    // Initialize IP pool manager
    let mut ip_manager = IpPoolManager::new();
    if load_config.user_resources.ip_binding.enabled {
        let pool_id = &load_config.user_resources.ip_binding.pool_id;
        let ip_pools: Vec<_> = scenario
            .workbook
            .ip_pools
            .iter()
            .filter(|p| &p.id == pool_id)
            .cloned()
            .collect();

        if ip_pools.is_empty() {
            return Err(anyhow::anyhow!(
                "IP pool '{}' not found in workbook",
                pool_id
            ));
        }

        ip_manager.initialize_from_config(&ip_pools)?;
        println!("✓ Initialized IP pool '{}'", pool_id);

        if let Some(stats) = ip_manager.get_stats(pool_id) {
            println!("  {}", stats);
        }
    }

    // Prepare user lifecycle parameters
    let iterations = load_config.user_lifetime.iterations;
    let think_time = parse_duration(&load_config.user_lifetime.think_time)?;

    // Collect all execution traces
    let mut all_traces = Vec::new();
    let mut user_id_counter = 0usize;

    println!("\n⏱️  Starting ramp-up...");

    // Execute users sequentially (no async in WASM component yet)
    for phase in &load_config.ramp_up.phases {
        println!(
            "\n📊 Phase at {}s: Spawning {} users...",
            phase.at_second, phase.spawn_users
        );

        for _ in 0..phase.spawn_users {
            user_id_counter += 1;
            let user_id = user_id_counter;

            // Determine tenant ID
            let tenant_id = phase
                .tenant_id
                .clone()
                .unwrap_or_else(|| "default-tenant".to_string());

            // Allocate IP if enabled
            let allocated_ip = if load_config.user_resources.ip_binding.enabled {
                let pool_id = &load_config.user_resources.ip_binding.pool_id;

                match ip_manager.allocate_ip(pool_id, &tenant_id, &format!("user-{}", user_id)) {
                    Ok(ip) => Some(ip),
                    Err(e) => {
                        eprintln!("⚠️  Failed to allocate IP for user-{}: {}", user_id, e);
                        None
                    }
                }
            } else {
                None
            };

            // Create user context
            let user_ctx = UserContext::new_with_id(user_id, tenant_id.clone(), allocated_ip);

            // Create user executor
            let mut executor = UserExecutor::new(
                user_ctx,
                scenario.workflows.clone(),
                scenario.actions.clone(),
                iterations,
                think_time,
            );

            // Create HTTP action component
            let mut component = HttpActionComponent::new();

            // Execute user
            match executor.run(&mut component) {
                Ok(traces) => {
                    println!(
                        "✓ User-{} completed {} iterations, {} actions",
                        user_id,
                        iterations,
                        traces.len()
                    );
                    all_traces.extend(traces);
                }
                Err(e) => {
                    eprintln!("✗ User-{} failed: {}", user_id, e);
                }
            }

            // Release IP if needed
            if load_config.user_resources.ip_binding.enabled {
                if let Some(ip) = allocated_ip {
                    let pool_id = &load_config.user_resources.ip_binding.pool_id;
                    if let Err(e) = ip_manager.release_ip(pool_id, ip) {
                        eprintln!(
                            "⚠️  Failed to release IP {} for user-{}: {}",
                            ip, user_id, e
                        );
                    }
                }
            }
        }
    }

    // Generate summary
    let mut summary = String::new();
    summary.push_str(&format!("\n📈 Load Test Summary\n"));
    summary.push_str("═══════════════════════════════════════\n");
    summary.push_str(&format!("Scenario: {}\n", scenario_name));
    summary.push_str(&format!("Total users spawned: {}\n", user_id_counter));
    summary.push_str(&format!("Total actions executed: {}\n", all_traces.len()));

    if !all_traces.is_empty() {
        // Calculate statistics
        let total_duration_ms: u64 = all_traces.iter().map(|t| t.duration_ms).sum();
        let avg_duration = total_duration_ms as f64 / all_traces.len() as f64;

        let mut durations: Vec<u64> = all_traces.iter().map(|t| t.duration_ms).collect();
        durations.sort_unstable();

        let p50 = durations[durations.len() / 2];
        let p95 = durations[durations.len() * 95 / 100];
        let p99 = durations[durations.len() * 99 / 100];

        summary.push_str("\nLatency Statistics:\n");
        summary.push_str(&format!("  Average: {:.2}ms\n", avg_duration));
        summary.push_str(&format!("  P50: {}ms\n", p50));
        summary.push_str(&format!("  P95: {}ms\n", p95));
        summary.push_str(&format!("  P99: {}ms\n", p99));
        summary.push_str(&format!("  Min: {}ms\n", durations[0]));
        summary.push_str(&format!("  Max: {}ms\n", durations[durations.len() - 1]));
    }

    // IP pool statistics
    if load_config.user_resources.ip_binding.enabled {
        let pool_id = &load_config.user_resources.ip_binding.pool_id;
        if let Some(stats) = ip_manager.get_stats(pool_id) {
            summary.push_str("\nIP Pool Statistics:\n");
            summary.push_str(&format!("  {}\n", stats));
        }
    }

    Ok(summary)
}

export!(SchedulerComponent);
