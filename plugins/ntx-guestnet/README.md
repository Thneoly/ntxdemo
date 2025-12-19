# ntx-guestnet

A strict, non-blocking guest networking stack extracted from `plugins/scheduler`.

This crate implements the **Guest-internal Packet → Flow → Transport → Socket** adapter with hard constraints:

- **Host interface is packet primitives only** (no host sockets).
- **Parsing/encoding lives in Transport only**.
- **Non-blocking** everywhere; backpressure is surfaced as `WouldBlock`.
- **Structured errors** for malformed packets (no stringly-typed parsing errors).

## Module layout

- `host_if`: shared memory view + host event/packet primitives (imports in component builds).
- `packet_io`: event/poll loop that drains packets from host and forwards payload as `PacketView`.
- `flow`: 5-tuple-ish keys, binding/lookup for sockets.
- `transport`: protocol state machines (UDP + RAW IPv4 TX) and packet (de)serialization.
- `socket_api`: user-facing socket API with non-blocking `send/recv`.
- `driver`: glue that drives RX (`PacketIo → SocketTable::on_packet`) and TX (`SocketTable::poll_tx → HostIf`).

## WIT

`wit/guestnet.wit` defines a minimal **library-level** WIT surface (no component glue yet):

- socket management: `create`, `bind-v4`, `connect-v4`
- non-blocking I/O: `recv-v4`, `send`
- RAW-specific: `set-raw-protocol`

### Zero-copy oriented datapath model

The WIT `host` interface is designed for a **zero-copy oriented** RX datapath:

- Host provides a `sharedmem` **resource** representing a read-only shared memory mapping.
- RX uses `packet-desc { payload: packet-slice { shm, offset, len }, ... }`.

This mirrors the Rust-side design:

- `host_if::PacketDesc { buf_offset, len, ... }`
- `host_if::PacketView<'a>` which borrows from `SharedMem::get_range()` without allocating.

Because canonical ABI details vary across runtimes, the WIT also keeps a **copy fallback**:

- `sharedmem.read(offset, len) -> list<u8>` (intended for debugging / compatibility, not the hot path).

### Socket semantics

The WIT mirrors the current Rust API semantics:

- UDP: `send(payload)` sends to an already-connected endpoint.
- RAW: `send(payload)` sends IPv4 **payload only** (IPv4 header is generated inside transport), and requires `set-raw-protocol` first.

## Testing

This crate keeps unit tests collocated as `*_tests.rs` and runs without a component runtime.

