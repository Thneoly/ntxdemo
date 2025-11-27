use std::{
    env,
    path::{Path, PathBuf},
};

use anyhow::Context;

use scheduler::SchedulerPipeline;

fn main() -> anyhow::Result<()> {
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

    Ok(())
}
