use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tokio::time::sleep;

use scheduler::core::dsl::{IpBindingStrategy, LoadSection};
use scheduler::{IpPoolManager, SchedulerPipeline, UserContext, UserExecutor, parse_duration};
use scheduler_actions_http::HttpActionComponent;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let default_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join("res/http_scenario.yaml");

    let scenario_path = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(default_path);

    let raw = std::fs::read_to_string(&scenario_path)
        .with_context(|| format!("failed to read scenario file: {}", scenario_path.display()))?;

    let mut pipeline = SchedulerPipeline::load_from_yaml_str(&raw)?;
    let summary = pipeline.summary();

    println!("scenario: {}", pipeline.scenario().name);
    println!("resources: {}", summary.resources);
    println!("metrics: {}", summary.metrics);
    println!("tasks: {}", summary.tasks);
    println!("transitions: {}", summary.edges);
    println!("resource ids:");
    for id in pipeline.workbook().resources.keys() {
        println!("  - {}", id);
    }

    // 检测是否有负载配置
    if let Some(load_config) = &pipeline.scenario().load {
        println!("\n🚀 Load testing mode enabled");
        println!("Ramp-up phases: {}", load_config.ramp_up.phases.len());
        println!("User lifetime: {:?}", load_config.user_lifetime.mode);
        println!("Iterations: {}", load_config.user_lifetime.iterations);
        println!("Think time: {}", load_config.user_lifetime.think_time);

        run_load_test(&pipeline, load_config).await?;
    } else {
        println!("\n📋 Single execution mode");
        let traces = pipeline.run_default()?;
        println!("executed {} action(s):", traces.len());
        for trace in traces {
            println!(
                "  - task={} action={} status={:?} detail={}",
                trace.task_id,
                trace.action_id,
                trace.status,
                trace.detail.as_deref().unwrap_or("<no detail>")
            );
        }
    }

    Ok(())
}

async fn run_load_test(pipeline: &SchedulerPipeline, load_config: &LoadSection) -> Result<()> {
    // 初始化 IP 池管理器
    let mut ip_manager = IpPoolManager::new();
    if load_config.user_resources.ip_binding.enabled {
        let pool_id = &load_config.user_resources.ip_binding.pool_id;
        let ip_pools: Vec<_> = pipeline
            .scenario()
            .workbook
            .ip_pools
            .iter()
            .filter(|p| &p.id == pool_id)
            .cloned()
            .collect();

        if ip_pools.is_empty() {
            anyhow::bail!("IP pool '{}' not found in workbook", pool_id);
        }

        ip_manager.initialize_from_config(&ip_pools)?;
        println!("✓ Initialized IP pool '{}'", pool_id);

        if let Some(stats) = ip_manager.get_stats(pool_id) {
            println!("  {}", stats);
        }
    }

    let ip_manager = Arc::new(Mutex::new(ip_manager));

    // 准备用户生命周期参数
    let iterations = load_config.user_lifetime.iterations;
    let think_time = parse_duration(&load_config.user_lifetime.think_time)?;

    // 收集所有执行痕迹
    let all_traces = Arc::new(Mutex::new(Vec::new()));

    // 用户计数器
    let mut user_id_counter = 0usize;
    let mut tasks = vec![];

    println!("\n⏱️  Starting ramp-up...");
    let start_time = Instant::now();

    // 按阶段生成用户
    for phase in &load_config.ramp_up.phases {
        let target_time = Duration::from_secs(phase.at_second);
        let elapsed = start_time.elapsed();

        if elapsed < target_time {
            sleep(target_time - elapsed).await;
        }

        println!(
            "\n📊 Phase at {}s: Spawning {} users...",
            phase.at_second, phase.spawn_users
        );

        // 为该阶段创建用户
        for _ in 0..phase.spawn_users {
            user_id_counter += 1;
            let user_id = user_id_counter;

            // 确定租户 ID
            let tenant_id = phase
                .tenant_id
                .clone()
                .unwrap_or_else(|| "default-tenant".to_string());

            // 分配 IP（如果启用）
            let allocated_ip = if load_config.user_resources.ip_binding.enabled {
                let pool_id = &load_config.user_resources.ip_binding.pool_id;
                let mut manager = ip_manager.lock().unwrap();

                match manager.allocate_ip(pool_id, &tenant_id, &format!("user-{}", user_id)) {
                    Ok(ip) => Some(ip),
                    Err(e) => {
                        eprintln!("⚠️  Failed to allocate IP for user-{}: {}", user_id, e);
                        None
                    }
                }
            } else {
                None
            };

            // 创建用户上下文
            let user_ctx = UserContext {
                id: user_id,
                tenant_id: tenant_id.clone(),
                allocated_ip,
                created_at: Instant::now(),
            };

            // 创建用户执行器
            let mut executor = UserExecutor::new(
                user_ctx,
                pipeline.scenario().workflows.clone(),
                pipeline.scenario().actions.clone(),
                iterations,
                think_time,
                pipeline.template_context().clone(),
            );

            // 克隆需要的变量
            let ip_manager_clone = Arc::clone(&ip_manager);
            let all_traces_clone = Arc::clone(&all_traces);
            let pool_id = load_config.user_resources.ip_binding.pool_id.clone();
            let ip_binding_enabled = load_config.user_resources.ip_binding.enabled;
            let release_on_task_end = matches!(
                load_config.user_resources.ip_binding.strategy,
                IpBindingStrategy::PerTask
            );

            // 启动用户任务
            let task = tokio::spawn(async move {
                // 创建 HTTP Action 组件
                let mut component = HttpActionComponent::new();

                match executor.run(&mut component) {
                    Ok(traces) => {
                        println!(
                            "✓ User-{} completed {} iterations, {} actions",
                            user_id,
                            iterations,
                            traces.len()
                        );

                        // 保存痕迹
                        let mut all = all_traces_clone.lock().unwrap();
                        all.extend(traces);
                    }
                    Err(e) => {
                        eprintln!("✗ User-{} failed: {:#}", user_id, e);
                    }
                }

                // 释放 IP（如果需要）
                if ip_binding_enabled && !release_on_task_end {
                    if let Some(ip) = allocated_ip {
                        let mut manager = ip_manager_clone.lock().unwrap();
                        if let Err(e) = manager.release_ip(&pool_id, ip) {
                            eprintln!(
                                "⚠️  Failed to release IP {} for user-{}: {}",
                                ip, user_id, e
                            );
                        }
                    }
                }
            });

            tasks.push(task);
        }
    }

    println!("\n⏳ Waiting for all users to complete...");

    // 等待所有用户任务完成
    for task in tasks {
        let _ = task.await;
    }

    let total_duration = start_time.elapsed();

    println!("\n📈 Load Test Summary");
    println!("═══════════════════════════════════════");
    println!("Total users spawned: {}", user_id_counter);
    println!("Total duration: {:.2}s", total_duration.as_secs_f64());

    let traces = all_traces.lock().unwrap();
    println!("Total actions executed: {}", traces.len());

    if !traces.is_empty() {
        // 计算统计信息
        let total_duration_ms: u64 = traces.iter().map(|t| t.duration_ms).sum();
        let avg_duration = total_duration_ms as f64 / traces.len() as f64;

        let mut durations: Vec<u64> = traces.iter().map(|t| t.duration_ms).collect();
        durations.sort_unstable();

        let p50 = durations[durations.len() / 2];
        let p95 = durations[durations.len() * 95 / 100];
        let p99 = durations[durations.len() * 99 / 100];

        println!("\nLatency Statistics:");
        println!("  Average: {:.2}ms", avg_duration);
        println!("  P50: {}ms", p50);
        println!("  P95: {}ms", p95);
        println!("  P99: {}ms", p99);
        println!("  Min: {}ms", durations[0]);
        println!("  Max: {}ms", durations[durations.len() - 1]);
    }

    // 显示 IP 池统计
    if load_config.user_resources.ip_binding.enabled {
        let pool_id = &load_config.user_resources.ip_binding.pool_id;
        let manager = ip_manager.lock().unwrap();
        if let Some(stats) = manager.get_stats(pool_id) {
            println!("\nIP Pool Statistics:");
            println!("  {}", stats);
        }
    }

    Ok(())
}
