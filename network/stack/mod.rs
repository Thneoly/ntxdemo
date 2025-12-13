//! Userspace network stack building blocks.
//!
//! The goal of this module is to keep **protocol processing** independent from the
//! underlying I/O backend (AF_PACKET today, AF_XDP later).

mod packet;
mod pipeline;
mod udp_echo;

pub use packet::*;
pub use pipeline::*;
pub use udp_echo::*;
