use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ActionCall {
    UdpSendReply,
    /// Forward-compat: allow new actions without breaking deserialization.
    #[serde(other)]
    Other,
}

impl ActionCall {
    pub(crate) fn as_call_str(&self) -> &'static str {
        match self {
            ActionCall::UdpSendReply => "udp.send-reply",
            ActionCall::Other => "other",
        }
    }
}
