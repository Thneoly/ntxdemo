# xdp-hello

## Prerequisites

1. Install a rust stable toolchain: `rustup install stable`
1. Install a rust nightly toolchain: `rustup install nightly`
1. Install bpf-linker: `cargo install bpf-linker`

## Build & Run

Use `cargo build`, `cargo check`, etc. as normal. Run your program with:

```shell
RUST_LOG=info cargo run --config 'target."cfg(all())".runner="sudo -E"'
```

### Note about `sudo` and `RUST_LOG`

On some systems, `sudo -E` may print:

> preserving the entire environment is not supported, `-E` is ignored

If that happens, `RUST_LOG=info ...` in front of `cargo run` might **not** be
preserved for the final `sudo target/debug/xdp-hello ...` process, which makes
it look like the program has “no output”.

Use this form instead (it passes `RUST_LOG` explicitly to the root process):

```shell
cargo build
sudo env RUST_LOG=info target/debug/xdp-hello --iface lo --mode skb
```

## Verify it's working (and not “stuck”)

This example is expected to **keep running** after it attaches the XDP program. It waits for Ctrl-C so the program stays attached.

### 1) Pick the right interface

The user-space loader defaults to `wlp10s0`.

If you're not sure what your interface is, list them:

```shell
ip link
```

For a quick local test, `lo` is usually the easiest:

```shell
RUST_LOG=info cargo run --config 'target."cfg(all())".runner="sudo -E"' -- --iface lo
```

### 2) Generate some traffic

In another terminal, send a few packets:

```shell
ping -c 3 127.0.0.1
```

You should see eBPF logs like `received a packet` printed by the running program.

### 3) If attach fails on Wi‑Fi, use SKB (generic) mode

Many Wi‑Fi drivers don't support native (driver) XDP.

Run with SKB mode explicitly:

```shell
RUST_LOG=info cargo run --config 'target."cfg(all())".runner="sudo -E"' -- --iface wlp10s0 --mode skb
```

Or just run in default `--mode auto` and the loader will try `SKB_MODE` as a fallback when attach fails.

### 4) Exit and cleanup

Press Ctrl-C in the running terminal. The loader will exit and the XDP program will be detached.
