use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};

/// Minimal VXLAN header (8 bytes).
///
/// We only parse enough to support "UDP payload contains an inner Ethernet frame":
/// - validate I flag (bit 3) is set
/// - extract the 24-bit VNI
///
/// Layout (8 bytes):
/// - flags: 8
/// - reserved1: 24
/// - vni: 24
/// - reserved2: 8
#[derive(Debug, Clone, Copy)]
pub struct Vxlan {
    pub vni: u32,
}

impl<'a> Layer<'a> for Vxlan {
    const ID: LayerId = LayerId::Vxlan;

    fn decode(input: &'a [u8]) -> Result<(Self, usize), String> {
        if input.len() < 8 {
            return Err("vxlan: truncated header".into());
        }

        let flags = input[0];
        // I flag = bit 3 (0x08)
        if (flags & 0x08) == 0 {
            return Err("vxlan: invalid flags (I flag not set)".into());
        }

        let vni = ((input[4] as u32) << 16) | ((input[5] as u32) << 8) | (input[6] as u32);
        Ok((Self { vni }, 8))
    }

    fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        out.clear();
        out.resize(8 + payload.len(), 0);
        out[0] = 0x08; // I flag
        // vni is 24-bit
        out[4] = ((self.vni >> 16) & 0xff) as u8;
        out[5] = ((self.vni >> 8) & 0xff) as u8;
        out[6] = (self.vni & 0xff) as u8;
        out[8..].copy_from_slice(payload);
    }

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        let Some(view) = ctx.abr.as_ref() else {
            return AcceptResult::Accept;
        };

        // If no VNI bindings configured, be permissive.
        if view.vni.is_empty() {
            return AcceptResult::Accept;
        }

        if view.vni.contains(self.vni) {
            AcceptResult::Accept
        } else {
            // Valid VXLAN but not for our active VNI resources.
            AcceptResult::Poison
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        // VXLAN always carries an inner Ethernet frame.
        Some(LayerId::Ether)
    }
}
