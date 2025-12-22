//! actions-executor 组件骨架（wasm32-wasip2）。
//! 当前仅回显输入，便于后续接入真实协议执行。

wit_bindgen::generate!({
    world: "scheduler:actions-executor/action-executor-component@0.2.0",
    path: [
        "../wit/core",
        "../wit/eventbus",
        "../wit/host",
        "../wit/protocol",
    ],
    generate_all,
    debug: true,
});

struct ActionExecutorImpl;

impl exports::scheduler::actions_executor::action_component::Guest for ActionExecutorImpl {
    fn init_component() -> Result<(), String> {
        println!("[actions-executor] init-component");
        Ok(())
    }

    fn execute_action(
        action: scheduler::core_types::types::ActionDef,
    ) -> Result<scheduler::core_types::types::ActionOutcome, String> {
        println!(
            "[actions-executor] execute action id={} call={} user={:?} task={:?}",
            action.id, action.call, action.user_id, action.task_id
        );
        Ok(scheduler::core_types::types::ActionOutcome {
            status: scheduler::core_types::types::ActionStatus::Success,
            detail: Some("stub executed".to_string()),
            latency_ms: None,
        })
    }

    fn release_component() -> Result<(), String> {
        println!("[actions-executor] release-component");
        Ok(())
    }
}

export!(ActionExecutorImpl);

