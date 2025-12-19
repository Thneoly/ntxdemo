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

pub mod host_if;

pub mod packet_io;

pub mod flow;

pub mod transport;

pub mod socket_api;

pub mod driver;

// Unit tests live next to the guestnet module to keep the layering clear.
#[cfg(test)]
mod packet_io_tests;

#[cfg(test)]
mod flow_tests;

#[cfg(test)]
mod transport_tests;

#[cfg(test)]
mod socket_api_tests;

#[cfg(test)]
mod driver_tests;

#[cfg(test)]
mod packet_io_injected_tests;

#[cfg(test)]
mod tx_tests;

// Next modules will be added in subsequent steps.
// pub mod packet_io;
// pub mod transport;
