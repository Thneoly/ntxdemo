# Scheduler Plugin

This directory hosts a mini workspace that assembles four focused crates into a runnable workflow scheduler plus a lightweight HTTP demo server. The scheduler still parses the DSL into a WBSTree → StateMachine pipeline, but the action runtime, HTTP actions, and core data structures now live in their own crates so they can be reused from wasm components or other hosts.

## 🚀 Quick Start

New to the project? Start here:

1. **[📖 QUICKSTART.md](doc/QUICKSTART.md)** - 5-minute quick start guide
2. **[📚 INDEX.md](doc/INDEX.md)** - Complete documentation index
3. **Run**: `./scripts/test_unified.sh` - Test the unified component

**Current Status**: ✅ Unified WebAssembly component available (430KB)

```bash
# Build the unified component
./scripts/create_unified.sh

# Test it
./scripts/test_unified.sh

# See what's possible
./scripts/compose_full.sh
```

## Workspace layout

| Crate | Description |
| --- | --- |
| `scheduler-core` | DSL parser, workbook/resource aggregation, WBSTree helpers, and the deterministic state machine. |
| `scheduler-executor` | Defines the `ActionComponent` lifecycle (`init → do_action → release`), `ActionContext`, and the event model used by the runtime. |
| `scheduler-actions-http` | Ships the default `HttpActionComponent` plus a simple logging component for tests. These call into the demo HTTP server so you can exercise real IO locally. |
| `scheduler` | Binaries (`scheduler`, `http_server`) and the priority-loop engine that wires the other crates together. |

Each crate also contains a `wit/` directory defining WebAssembly Component Model interfaces, enabling them to be compiled as standalone wasm32-wasip2 components for use in other runtimes or component compositions.

## Building as wasm components

To compile the scheduler crates as WebAssembly components:

```bash
# Build all components at once (currently only core-libs is functional)
./scripts/build_all_components.sh

# Or build individually
cd core-libs && ./build.sh
cd executor && ./build.sh    # 🚧 In progress
cd actions-http && ./build.sh  # 🚧 In progress
```

**Demo with working component:**
```bash
./scripts/compose_demo.sh
```

Component outputs are generated in:
- `core-libs/target/wasm32-wasip2/release/scheduler_core.wasm` ✅
- `executor/target/wasm32-wasip2/release/scheduler_executor.wasm` 🚧
- `actions-http/target/wasm32-wasip2/release/scheduler_actions_http.wasm` 🚧

**Requirements**: `cargo component` and `wasm-tools` must be installed:
```bash
cargo install cargo-component wasm-tools
```

## Component Composition

The `scheduler-core` component can be used immediately in compositions:

```bash
# Inspect the component interface
wasm-tools component wit target/wasm32-wasip2/release/scheduler_core.wasm

# Validate the component
wasm-tools validate target/wasm32-wasip2/release/scheduler_core.wasm
```

See `examples/composition.wac` for a WAC composition example. Full multi-component composition will be available once executor and actions-http are complete.

## Running the composed component with Wasmtime

After generating `wac/scheduler-composed.wasm` (for example via `wac/scheduler-composition.wac`), you can invoke the exported `run-scenario` function directly from Wasmtime. The helper script below loads a scenario file, encodes it as a WAVE multiline string, and forwards it to the component with the required network capabilities enabled.

```bash
# Default: uses res/http_scenario.yaml and wac/scheduler-composed.wasm
./scripts/run_scheduler_component.sh

# Custom scenario/component
./scripts/run_scheduler_component.sh res/simple_scenario.yaml wac/scheduler-composed.wasm
```

Under the hood the script expands to:

```bash
WASMTIME_BACKTRACE_DETAILS=1 \
wasmtime run \
	--wasi tcp=y \
	--wasi inherit-network=y \
	--invoke 'run-scenario("""
		# YAML contents (escaped as a WAVE multiline string)
	""")' \
	wac/scheduler-composed.wasm
```

> ℹ️ The `--invoke` flag uses the [WAVE](https://github.com/bytecodealliance/wasm-tools/tree/main/crates/wasm-wave) text format for component values. Wrapping the YAML payload in a triple-quoted string (`""" ... """`) preserves newlines so `run-scenario` receives the exact scenario text.

## WIT interface design

Each crate exports typed interfaces defined in its `wit/world.wit`:

- **core-libs** exports `scheduler:core-libs/types` (Scenario, ActionDef, WorkflowNode, etc.) and `scheduler:core-libs/parser` (parse/validate functions).
- **executor** exports `scheduler:executor/types` (ActionOutcome, WbsTask, etc.), `scheduler:executor/context` (ActionContext resource), and `scheduler:executor/component-api`.
- **actions-http** exports `scheduler:actions-http/http-component` (init/do_http_action/release functions).

These interfaces allow other wasm runtimes to link or compose scheduler functionality without a full native Rust build.

## 📚 Documentation

### Quick Links

- **[📖 INDEX.md](doc/INDEX.md)** - Complete documentation index
- **[🚀 QUICKSTART.md](doc/QUICKSTART.md)** - 5-minute quick start
- **[📊 SUMMARY.md](doc/SUMMARY.md)** - Project status and achievements
- **[🏗️ ARCHITECTURE.md](doc/ARCHITECTURE.md)** - Architecture diagrams
- **[🔧 WAC_COMPOSITION.md](doc/WAC_COMPOSITION.md)** - Technical details
- **[📦 USAGE.md](doc/USAGE.md)** - API integration guide
- **[📁 FILE_INDEX.md](doc/FILE_INDEX.md)** - File reference
- **[📂 DIRECTORY_STRUCTURE.md](doc/DIRECTORY_STRUCTURE.md)** - Directory organization

### Component Commands

```bash
# Build unified component (430KB, includes core-libs)
./scripts/create_unified.sh

# Test and validate component
./scripts/test_unified.sh

# View full composition plan
./scripts/compose_full.sh
```

## Binaries

| Binary | Description |
| --- | --- |
| `scheduler` | CLI that loads a scenario (defaults to `res/http_scenario.yaml`), prints the parsed summary, and executes the workflow via the default `HttpActionComponent` from `scheduler-actions-http`. |
| `http_server` | Minimal HTTP endpoint (configurable via YAML) that responds to `/asset`, `/get`, `/post`, `/json`, and `/health` so scheduler workflows can be tested locally. |

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

For more realistic local tests, load a config file (YAML or JSON) that sets the listen address and default payloads:

```bash
# Uses res/http_server_config.yaml to mirror the scheduler's expectations
cargo run --bin http_server -- --config res/http_server_config.yaml

# Equivalent environment variable
HTTP_SERVER_CONFIG=res/http_server_config.yaml cargo run --bin http_server
```

`res/http_server_config.yaml` ships with sensible defaults:

- `listen_addr`: socket to bind (defaults to `127.0.0.1:8080`).
- `asset`: overrides the `/asset` metadata, expected POST response status, and the JSON echoed under `expected_asset`.
- `responses.json` / `responses.health`: arbitrary payloads returned under the `payload` key for `/json` and `/health`.

`GET /asset` returns JSON containing `ip`, `port`, and the configured `status_code` (default 200). `POST /asset` echoes any JSON body under the `result` field and includes the expected asset shape from the config under `expected_asset`, allowing the scheduler workflow to verify local expectations.

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
