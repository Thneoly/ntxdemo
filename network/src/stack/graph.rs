use super::layer::{LayerId, LayerInstance};

/// Edge kind between layer nodes.
///
/// Today we only build a linear chain (Encapsulates), but this enum allows extending
/// to tunneling and multi-payload cases later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Upper layer encapsulates the lower layer (classic L2->L3->L4).
    Encapsulates,
    /// Upper layer's payload contains an inner packet (e.g. VXLAN/GRE/Geneve).
    Tunnels,
}

/// A graph representation of a decoded packet.
///
/// MVP: this is a chain encoded as graph nodes 0..n-1 with edges i -> i+1.
///
/// Future: a tunnel layer can add a second "inner" chain starting from its payload,
/// producing a real graph.
#[derive(Debug, Default)]
pub struct PacketGraph<'a> {
    nodes: Vec<LayerInstance>,
    edges: Vec<(usize, usize, EdgeKind)>,
    payload: &'a [u8],
}

impl<'a> PacketGraph<'a> {
    pub fn new(
        nodes: Vec<LayerInstance>,
        edges: Vec<(usize, usize, EdgeKind)>,
        payload: &'a [u8],
    ) -> Self {
        Self {
            nodes,
            edges,
            payload,
        }
    }

    pub fn nodes(&self) -> &[LayerInstance] {
        &self.nodes
    }

    pub fn edges(&self) -> &[(usize, usize, EdgeKind)] {
        &self.edges
    }

    pub fn payload(&self) -> &'a [u8] {
        self.payload
    }

    pub fn find(&self, id: LayerId) -> Option<&LayerInstance> {
        self.nodes.iter().find(|n| n.id == id)
    }
}
