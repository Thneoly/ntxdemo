# Scheduler Plugin

This crate parses workflow DSL scenarios into a WBSTree → StateMachine pipeline and now also ships with a lightweight HTTP test server for local experiments.

## Binaries

| Binary | Description |
| --- | --- |
| `scheduler` | CLI that loads a scenario (defaults to `res/http_scenario.yaml`), prints the parsed summary, and executes the workflow via the default action component. |
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

- `SchedulerPipeline::run` accepts any `ActionComponent` implementation that exposes the lifecycle `init → do_action → release`. The built-in `DefaultActionComponent` just logs the `call` field, but custom components can perform real IO or maintain external resources.
- Every component receives an `ActionContext`, which exposes helpers to register new actions, add/remove/update tasks, and edit edges. Each mutating call automatically re-syncs the FSM so downstream transitions stay consistent.
- The runtime loop monitors the WBSTree for newly inserted tasks and will execute them in the same session, enabling dynamic fan-out workflows.

## Usage walkthrough

1. **Prepare the HTTP demo target**
	- Run `cargo run --bin http_server` to bring up the sample `/asset` endpoint (or point the scenario to your own service).
2. **Execute the scheduler runtime**
	- Run `cargo run` inside this crate. The CLI will load `res/http_scenario.yaml`, print a structural summary, then execute every action using the default component. Execution traces print the task ID, action ID, and status/detail for each step.
3. **Customize execution**
	- Implement `ActionComponent` to call real services or inject dynamic tasks. Components can allocate resources during `init`, perform the actual RPC/logic in `do_action`, and cleanup handles in `release`. Pass your component to `SchedulerPipeline::run` (see `src/engine.rs` tests for an example). Any tasks inserted during execution will be picked up automatically.

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
