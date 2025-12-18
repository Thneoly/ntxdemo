use crate::TcpHeader;
use crate::stack::{AcceptResult, Layer, LayerId, PacketContext};

#[derive(Debug, Clone, Copy)]
pub struct Tcp {
    pub src_port: u16,
    pub dst_port: u16,
    pub data_offset_bytes: usize,
}

impl<'a> Layer<'a> for Tcp {
    const ID: LayerId = LayerId::Tcp;

    fn decode(data: &'a [u8]) -> Result<(Self, usize), String> {
        let (hdr, _payload) = TcpHeader::decode(data).map_err(|e| e.to_string())?;
        let header_len = hdr.header_len();
        Ok((
            Self {
                src_port: hdr.src_port,
                dst_port: hdr.dst_port,
                data_offset_bytes: header_len,
            },
            header_len,
        ))
    }

    fn encode(&self, payload: &[u8], out: &mut Vec<u8>) {
        // MVP: TCP encode is intentionally minimal; we don't have an existing high-level
        // builder with checksum and flags here like UDP. For now, just preserve payload.
        out.clear();
        out.extend_from_slice(payload);
    }

    fn accept(&self, ctx: &PacketContext) -> AcceptResult {
        let Some(view) = ctx.abr.as_ref() else {
            return AcceptResult::Accept;
        };

        // We currently don't carry IPv4 dst in the TCP layer, so we can't do ip+port matching
        // here. In ABR terms, this means we can only enforce a wildcard (0.0.0.0, port) bind.
        //
        // Callers that want strict (ip, port) enforcement should:
        // - either extend the TCP layer to carry dst_ip (like UDP does), or
        // - enforce at IPv4 layer before parsing TCP.
        if view.tcp_ports.contains_be(0, self.dst_port) {
            AcceptResult::Accept
        } else {
            AcceptResult::Poison
        }
    }

    fn next_layer(&self) -> Option<LayerId> {
        None
    }
}
