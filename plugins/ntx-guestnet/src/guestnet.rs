//! Guest-internal Packet → Flow → Transport → Socket adapter (strict layering).
//!
//! Architecture (do not collapse layers):
//! Application
//!   └─ socket_api  (library-level API)
//!      └─ transport (FSM + packet parse/generate)
//!         └─ flow    (5-tuple + binding + lookup)
//!            └─ packet_io (poll_packet loop)
//!               └─ host_if (shared-mem + events)
//!
//! Notes for maintainers (ABCD hardening):
//! - Backpressure must be explicit: transport RX queue returns `WouldBlock`, and socket-pump
//!   pressure is observable via `PumpReport`/`DriveReport::Pump`.
//! - Packet malformation errors are structured (`MalformedPacketReason`) to avoid string/ABI churn.
//! - Flow lifecycle/last_seen is owned by Transport; socket pump must not mutate flow state.
//! - TX path is intentionally not wired to Host IF yet; when added, it must remain packet-primitive
//!   only (no host sockets) and keep generation inside Transport.

// NOTE: In this extracted crate we use a flat `src/` module layout declared in `lib.rs`.
// This file is retained for documentation and architectural notes only.
