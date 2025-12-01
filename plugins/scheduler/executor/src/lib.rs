wit_bindgen::generate!({
    world: "scheduler-executor",
    path: "wit",
    generate_all,
    debug: true,
});

use exports::scheduler::executor::component_api;
use exports::scheduler::executor_types::types::{
    ActionContext, ActionDef, ActionOutcome, ActionStatus,
};
use scheduler::executor::context_helper;

struct SchedulerExecutorImpl;

impl component_api::Guest for SchedulerExecutorImpl {
    fn execute_action(action: ActionDef, ctx: ActionContext) -> Result<ActionOutcome, String> {
        // Demonstrate interaction with the scheduler-provided context helpers to ensure
        // bindings remain plumbed, even though the actual logic is still a stub.
        let detail = if let Some(task) = context_helper::get_task(ctx, &action.id) {
            format!(
                "executor inspected existing task={} for action={}",
                task.id, action.id
            )
        } else {
            context_helper::register_action(ctx, &action);
            format!(
                "executor registered action={} call={} (stub)",
                action.id, action.call
            )
        };

        Ok(ActionOutcome {
            status: ActionStatus::Success,
            detail: Some(detail),
        })
    }
}

export!(SchedulerExecutorImpl);
