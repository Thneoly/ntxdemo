# Runner MVP WIT Package

This directory captures the interface layout for the MVP version of Runner's `Core + ProtocolFrame + Protocol` component. The definitions follow the conventions documented in `../wit.md` and rely on WASI Preview2 packages pinned through `plugins/runner/wit/deps.toml`.

## Files

| File | Purpose |
| --- | --- |
| `types.wit` | Shared records/enums (tasks, actions, rate profiles, scheduler errors, PF capabilities). |
| `core-libs.wit` | Runtime capabilities Core exposes (logger, timer, sockets, random, call-model, progress) with explicit WASI imports. |
| `core-scheduler.wit` | Scheduler surface used to initialize workflows, poll actions, complete results, and enqueue protocol-driven tasks. |
| `protocol-frame.wit` | Bridge contract between Core and Protocol implementations (task claim/commit, context store, progress, dynamic tasks). |
| `protocol.wit` | Minimal interface a protocol component must implement (`init`, `run-action`, `on-error`, `release`). |

## Generating bindings

1. Ensure the WASI dependencies in `plugins/runner/wit/deps.toml` are fetched:

   ```bash
   cd plugins/runner/wit
   wit-deps update
   ```

2. Run `wit-bindgen` (or `wasm-tools component wit`) against this directory to prototype host/guest bindings:

   ```bash
   wit-bindgen rust --world core-runtime ../../docs/plan/mvp/wit
   ```

   Adjust the `--world` flag (`scheduler-world`, `protocol-frame-world`, `protocol-world`) depending on the target module you are validating.

These schemas are reference designs for the MVP planning docs; implementations in `plugins/runner/wit/` can evolve independently but should keep these contracts in sync after design reviews.
