# packet-guest (wasm32-wasip2 component)

This crate is a small demo WebAssembly **component** that exports a packet-engine ABI:

- `desc-get/desc-put` for the descriptor ring buffer
- `payload-get/payload-put` for payload bytes
- `notify-rx` to consume queued descriptors

## Build outputs

### Fast path (what we use in this repo)

This repository is set up so that a plain Rust build already emits a **component**-encoded binary for `wasm32-wasip2`:

```bash
cargo build -p packet-guest --target wasm32-wasip2
```

The artifact is produced at:

- `target/wasm32-wasip2/debug/packet_guest.wasm`

You can verify it is a component with:

```bash
wasm-tools component wit target/wasm32-wasip2/debug/packet_guest.wasm
```

### Optional: `cargo-component` (if you want a dedicated component workflow)

If you prefer the explicit component build workflow, install the tool:

```bash
cargo install cargo-component
```

Then build:

```bash
cargo component build -p packet-guest
```

Notes:

- `cargo-component` is optional for this repo because `wasm32-wasip2` already produces a component-shaped binary.
- If you change your toolchain and the output becomes a *core module* again, you can wrap it with `wasm-tools component new`.
# packet-guest

A minimal wasm32-wasip2 component used to demo host<->guest shared-memory packet delivery.

Exports two memories:
- `desc`: control block + descriptor ring
- `payload`: raw bytes

Exports `notify-rx()` which consumes descriptors written by the host.
