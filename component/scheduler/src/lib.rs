//! 调度器组件骨架（wasm32-wasip2）。
//! 仅提供占位实现，便于后续对接状态机与负载控制逻辑。

wit_bindgen::generate!({
    world: "scheduler:main/scheduler-main@0.2.0",
    path: [
        "../wit/core",
        "../wit/eventbus",
        "../wit/host",
        "../wit/protocol",
        "../wit/scheduler",
    ],
    generate_all,
    debug: true,
});

struct SchedulerExports;

impl exports::scheduler::main::scheduler_component::Guest for SchedulerExports {
    fn run(config_dir: String) -> Result<(), String> {
        // 占位：真实实现应在此加载 workflow/workbook/load 配置并进入事件循环。
        println!("[scheduler] run with config dir: {config_dir}");
        Ok(())
    }
}

impl exports::scheduler::main::send_scheduler::Guest for SchedulerExports {
    fn schedule_send(
        request: exports::scheduler::main::send_scheduler::SendRequest,
    ) -> Result<String, String> {
        // 直接回显 request-id，后续可接入计时器/速率控制。
        Ok(request.request_id.clone())
    }

    fn cancel_send(_request_id: String) -> Result<(), String> {
        Ok(())
    }

    fn query_send_status(request_id: String) -> Result<
        exports::scheduler::main::send_scheduler::SendStatus,
        String,
    > {
        Ok(exports::scheduler::main::send_scheduler::SendStatus {
            request_id,
            state: exports::scheduler::main::send_scheduler::SendRequestState::Pending,
            total_sent: 0,
            last_sent_time: None,
            next_send_time: None,
        })
    }
}

export!(SchedulerExports);

