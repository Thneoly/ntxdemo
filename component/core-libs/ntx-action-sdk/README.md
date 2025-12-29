# ntx-action-sdk

Small shared helper crate ("frame"/SDK) for Ntx scenario components.

It centralizes conventions that otherwise get copied across components:

- `payload` / `payload_hex` / `payload_bytes` parsing
- scheduling JSON parsing (once / periodic / timetable / rate-limited)
- publishing `packet.tx-request` and `send.schedule-request`

This crate is intentionally dependency-light and suitable for linking into wasm32-wasip2 component crates.

## Framework mode (traits/macros)

If you want this as a **framework** (stronger structure, less copy/paste), use:

- `ActionRequest` / `ActionCtx`: component-agnostic request/context
- `EventBusAdapter`: lets a component plug in its WIT-generated event type
- `ActionRuntime`: provides standard helpers (e.g. `publish_tx_request`)
- `ActionModule`: a single entrypoint for all action handling
- `declare_actions!`: dispatch macro to enforce a consistent routing pattern

The component crate remains responsible for adapting WIT types into these framework types.
