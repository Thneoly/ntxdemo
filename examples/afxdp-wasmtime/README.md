# afxdp-wasmtime (AF_XDP + shared UMEM + Wasmtime guest)

This example demonstrates the *data path*:

NIC → XDP (redirect) → AF_XDP (XSK) → **UMEM (shared pages)** → Rust host → Wasmtime guest

Today the host calls the guest with `list<u8>` (portable). The **kernel-to-userspace path is still zero-copy** (DMA into UMEM). We can evolve the guest boundary into a true shared-memory view once we settle on the exact Wasmtime sharing strategy.

## Prerequisites

- Linux kernel with AF_XDP support
- Root privileges (XDP attach + XSK bind)
- Rust toolchain
- bpf-linker:

```bash
cargo install bpf-linker
```

For the guest (wasip2):

```bash
cargo install cargo-component wasm-tools
```

## Build

From `examples/afxdp-wasmtime`:

```bash
# build guest component
(cd afxdp-guest && cargo component build --release)

# build host (also builds embedded eBPF)
(cd afxdp-host && cargo build)
```

## Run

### Loopback quick test

Attach XDP to `lo` queue 0 and print basic parsing results:

```bash
sudo -E RUST_LOG=info \
  ./afxdp-host/target/debug/afxdp-host \
  --iface lo --queue 0 \
  --guest ../target/wasm32-wasip1/release/afxdp_guest.wasm
```

In another terminal:

```bash
ping -c 3 127.0.0.1
```

### Notes

- Some devices (especially Wi‑Fi) require `--mode skb`.
- AF_XDP needs a real RX queue; on some virtual/loopback paths you might not see packets depending on kernel setup.

### Fallback: SKB/generic + COPY (functional demo)

On some NIC/driver combos (e.g. Realtek `r8169` on RTL8126), **native/driver XDP attach and/or AF_XDP zero-copy are not supported**.
In that case you can still demonstrate the functional chain of:

XDP hit/redirect accounting → userspace host observability → Wasm component load

Run the host explicitly in SKB mode and request COPY bind:

```bash
sudo env RUST_LOG=info \
  ./target/debug/afxdp-host \
  --iface eno1 --queue 0 \
  --mode skb \
  --xsk-bind copy \
  --poll-rx --poll-timeout-ms 50 \
  --guest ./target/wasm32-wasip1/release/afxdp_guest.wasm
```

Generate traffic (examples):

```bash
ping -c 3 -I eno1 192.168.1.1
```

What “success” looks like in this fallback mode:

- the log line `xdp: hit+... redir_ok+...` increases every second
- **it is expected that `rx: total=0 pps=0` may remain 0 on unsupported hardware**

Additionally, the host will call into the Wasm guest:

- in fallback mode, it calls the guest export `run()` once per second when XDP hit counters increase
- when AF_XDP RX is supported and frames are delivered to userspace, the host calls the guest export `packet.on-packet(meta, data)` per received frame

Note:

- If you only see the fallback `run()` calls but never `packet.on-packet`, that means your NIC/driver can run XDP in SKB/generic mode but does not deliver frames to AF_XDP on this setup (common on unsupported hardware).

### Fallback: SKB/generic + AF_PACKET (real bytes into guest)

If AF_XDP does not deliver frames to userspace on your hardware (so `rx: total=0` stays 0), you can still demonstrate the **end-to-end** “real frame bytes → userspace → Wasm guest” chain using an AF_PACKET raw socket (copy).

Run the host in SKB mode and switch capture to `afpacket`:

```bash
sudo env RUST_LOG=info \
  ./target/debug/afxdp-host \
  --iface eno1 --queue 0 \
  --mode skb \
  --capture afpacket \
  --guest ./target/wasm32-wasip1/release/afxdp_guest.wasm
```

Generate some traffic (examples):

```bash
ping -c 3 -I eno1 192.168.1.1
```

What “success” looks like:

- you see `afxdp-guest: on_packet(...)` (called once per captured frame)
- XDP stats still increment (it’s fine if `redir_ok` remains low/0 on unsupported AF_XDP setups)

Notes:

- AF_PACKET capture requires root.
- This is a copy path; it’s intended as a functional demo on NICs that can’t do AF_XDP RX.

## What’s inside

- `afxdp-ebpf`: XDP program with `XSKS` map and `redirect()` to AF_XDP sockets
- `afxdp-host`: userspace host that creates UMEM+XSK and updates XSK map, then runs Wasmtime guest
- `afxdp-guest`: wasip2 component; parses Ethernet/IPv4/TCP and prints an HTTP heuristic

```shell
sudo apt update
sudo apt install -y \
  build-essential pkg-config \
  clang llvm \
  libelf-dev zlib1g-dev

sudo apt install -y \
  linux-headers-$(uname -r) \
  bpftool


sudo env RUST_LOG=info \
  ./target/debug/afxdp-host \
  --iface eno1 --queue 0 \
  --mode skb \
  --capture afpacket \
  --sample 50 \
  --guest ./target/wasm32-wasip1/release/afxdp_guest.wasm
```