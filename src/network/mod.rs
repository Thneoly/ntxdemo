// Re-export the standalone `ntx-network` library crate through the existing
// `crate::network::*` path, to minimize churn in the main crate.

pub use ntx_network::*;
