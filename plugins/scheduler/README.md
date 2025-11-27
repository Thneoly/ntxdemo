# Scheduler Plugin

This directory hosts a mini workspace that assembles four focused crates into a runnable workflow scheduler plus a lightweight HTTP demo server. The scheduler still parses the DSL into a WBSTree → StateMachine pipeline, but the action runtime, HTTP actions, and core data structures now live in their own crates so they can be reused from wasm components or other hosts.

## Workspace layout

| Crate | Description |
| --- | --- |
| `scheduler-core` | DSL parser, workbook/resource aggregation, WBSTree helpers, and the deterministic state machine. |
| `scheduler-executor` | Defines the `ActionComponent` lifecycle (`init → do_action → release`), `ActionContext`, and the event model used by the runtime. |
| `scheduler-actions-http` | Ships the default `HttpActionComponent` plus a simple logging component for tests. These call into the demo HTTP server so you can exercise real IO locally. |
| `scheduler` | Binaries (`scheduler`, `http_server`) and the priority-loop engine that wires the other crates together. |

## Binaries

| Binary | Description |
| --- | --- |
| `scheduler` | CLI that loads a scenario (defaults to `res/http_scenario.yaml`), prints the parsed summary, and executes the workflow via the default `HttpActionComponent` from `scheduler-actions-http`. |
| `http_server` | Minimal HTTP endpoint that responds to `GET /asset` and `POST /asset`, mirroring the sample scenario requirements. |

## Try it

```bash
# Start the HTTP test server on the default 127.0.0.1:8080
cargo run --bin http_server

# In another shell, run the scheduler summary + runtime (reads res/http_scenario.yaml)
cargo run
```

`http_server` accepts either a positional socket address or the `HTTP_TEST_ADDR` environment variable. Example:

```bash
cargo run --bin http_server -- 0.0.0.0:9000
# or
HTTP_TEST_ADDR=0.0.0.0:9000 cargo run --bin http_server
```

`GET /asset` returns JSON containing `ip`, `port`, and `status_code = 200`. `POST /asset` echoes any JSON body under the `result` field so the scheduler workflow can assert against it.

## Runtime hooks

- `SchedulerPipeline::run` accepts any `ActionComponent` implementation defined in `scheduler-executor`. The default CLI calls `run_default`, which instantiates the HTTP component so you get a working demo out of the box, but you can pass your own component to integrate with real services.
- The executor is a single-threaded `loop {}` with 64 priority lanes (0 = highest, 63 = lowest). Actions are wrapped as tasks (default priority 32), WBS mutations become higher-priority **events** (priority 4), and an `idle` task (priority 63) runs whenever the queues are empty so the loop never spins tight.
- `ActionContext` now enqueues those events instead of mutating the WBSTree directly. When the queued event task runs, it applies the change and re-syncs the FSM, guaranteeing consistent state even when many actions mutate the workflow concurrently.
- The runtime keeps scanning for newly inserted action tasks after every action/event, so dynamic fan-out workflows continue in the same session. Hitting <kbd>Ctrl+C</kbd> (or sending `SIGINT`) flips a shutdown flag and the loop exits gracefully after the current task completes.

## Extending actions or building components

- Create a new crate next to `scheduler-actions-http` and implement `ActionComponent` for your domain. Because the executor crate has zero HTTP-specific dependencies, your component can talk to anything (databases, queues, device bridges, etc.).
- If you plan to publish the component as a wasm module, keep the component crate `no_std`-friendly and compile it with `cargo component` targeting `wasm32-wasip2`. The scheduler binary can embed the wasm runtime later, or you can deploy the component into another host entirely.
- For quick experiments, you can also reuse the logging component in `scheduler-actions-http::LoggingActionComponent`, which simply prints the call metadata and succeeds.

## Task scheduler details

| Concept | Description |
| --- | --- |
| Priority lanes | Fixed array of 64 queues. Smaller numbers run first; ties preserve FIFO order within a lane. |
| Action task | Wraps a WBS node with `action_id`. Default priority = 32 but can be adjusted when constructing a `ScheduledTask`. |
| Event task | Represents `SchedulerEvent` emitted by `ActionContext` (register/add/remove/update). Uses priority = 4 so mutations are applied before subsequent actions. |
| Idle task | Automatically injected (priority = 63) when all queues are empty; performs a short sleep (10 ms) to avoid hot spinning. Two consecutive idle spins without new work will end the loop unless a shutdown signal is pending. |
| Shutdown flag | A shared `AtomicBool` toggled by the Ctrl+C handler; once set, the loop finishes the current task/event and returns the collected traces. |

> TIP: If you need domain-specific priorities (e.g., “probe before push”), you can fork `ScheduledTask::action` to accept a custom `priority: u8` and propagate it through the DSL. The executor already enforces ordering across lanes.

## Usage walkthrough

1. **Prepare the HTTP demo target**
	- Run `cargo run --bin http_server` to bring up the sample `/asset` endpoint (or point the scenario to your own service).
2. **Execute the scheduler runtime**
	- Run `cargo run` inside this crate (or `cargo run -p scheduler --bin scheduler` from the workspace root). The CLI will load `res/http_scenario.yaml`, print a structural summary, then execute every action using the default HTTP component. Execution traces print the task ID, action ID, and status/detail for each step. Press <kbd>Ctrl+C</kbd> at any time to request a graceful shutdown; the runtime finishes the current task/event and flushes traces.
3. **Customize execution**
	- Implement `ActionComponent` to call real services or inject dynamic tasks. Components can allocate resources during `init`, perform the actual RPC/logic in `do_action`, and cleanup handles in `release`. Use the `ActionContext` helpers to enqueue events (register actions, add/remove tasks, edit edges). Pass your component to `SchedulerPipeline::run` (see `scheduler/src/engine.rs` tests for an example) and the priority loop will pick up any tasks that those events add, preserving ordering guarantees between action/event lanes.

## Testing

```bash
# Run all unit tests (DSL, WBS, FSM, component runtime, CLI) and the HTTP server handler tests
cargo test
```

`cargo test` exercises:
- DSL parsing/validation (including error paths).
- WBSTree CRUD helpers and edge preservation.
- StateMachine sync/remove behavior.
- Workbook metric/resource aggregation.
- Scheduler pipeline runtime (ensuring components can spawn dynamic tasks).
- HTTP server bin handler responses for both GET and POST.
