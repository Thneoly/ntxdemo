use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ResourceType {
    UdpEndpoint,
    /// Forward-compat: allow new resource types without breaking deserialization.
    #[serde(other)]
    Other,
}
