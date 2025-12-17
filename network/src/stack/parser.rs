use super::graph::{EdgeKind, PacketGraph};
use super::layer::{LayerId, LayerInstance};
use super::registry::LayerRegistry;

/// The "never changes" parsing loop.
///
/// New protocol support = implement a new `Layer` + register in `LayerRegistry`.
pub fn parse_packet<'a>(
    data: &'a [u8],
    first: LayerId,
    registry: &LayerRegistry,
) -> Result<(Vec<LayerInstance>, &'a [u8]), String> {
    let mut layers = Vec::new();
    let mut offset = 0usize;
    let mut current = Some(first);

    while let Some(id) = current {
        if offset > data.len() {
            return Err("offset beyond input".into());
        }
        let (layer, used, next_hint, bind_key) = registry.decode(id, &data[offset..])?;
        offset = offset
            .checked_add(used)
            .ok_or_else(|| "offset overflow".to_string())?;

        // Hybrid next-layer selection:
        // 1) layer-chosen `next_hint` (fast path)
        // 2) registry-driven binding (Scapy-like bind_layers)
        current = match (next_hint, bind_key) {
            (Some(n), _) => Some(n),
            (None, Some(key)) => registry.resolve_binding(id, key),
            (None, None) => None,
        };
        layers.push(layer);
        if offset == data.len() {
            break;
        }
    }

    Ok((layers, &data[offset..]))
}

/// Variant of `parse_packet` that also returns per-layer spans (start,end) within `data`.
///
/// This is useful for graph/tunnel building to locate a layer's payload without re-decoding.
pub fn parse_packet_with_spans<'a>(
    data: &'a [u8],
    first: LayerId,
    registry: &LayerRegistry,
) -> Result<(Vec<LayerInstance>, Vec<(usize, usize)>, &'a [u8]), String> {
    let mut layers = Vec::new();
    let mut spans = Vec::new();
    let mut offset = 0usize;
    let mut current = Some(first);

    while let Some(id) = current {
        if offset > data.len() {
            return Err("offset beyond input".into());
        }
        let start = offset;
        let (layer, used, next_hint, bind_key) = registry.decode(id, &data[offset..])?;
        offset = offset
            .checked_add(used)
            .ok_or_else(|| "offset overflow".to_string())?;
        let end = offset;

        current = match (next_hint, bind_key) {
            (Some(n), _) => Some(n),
            (None, Some(key)) => registry.resolve_binding(id, key),
            (None, None) => None,
        };

        layers.push(layer);
        spans.push((start, end));

        if offset == data.len() {
            break;
        }
    }

    Ok((layers, spans, &data[offset..]))
}

/// Symmetric builder loop: encode from the innermost layer outwards.
pub fn build_packet(
    layers: &[LayerInstance],
    payload: &[u8],
    registry: &LayerRegistry,
) -> Result<Vec<u8>, String> {
    // MVP implementation uses a temporary buffer and clones payload per layer.
    // This can be upgraded later to a two-buffer swap or scatter-gather.
    let mut buf = payload.to_vec();
    let mut out = Vec::new();

    for layer in layers.iter().rev() {
        out.clear();
        registry.encode(layer.id, &*layer.inner, &buf, &mut out)?;
        buf.clear();
        buf.extend_from_slice(&out);
    }

    Ok(buf)
}

/// Convenience wrapper over [`build_packet`] that defaults the innermost payload to empty.
///
/// This keeps the API ergonomic for the common case (no application payload), while still
/// requiring callers to pass an explicit registry (no hidden global / allocation).
pub fn build_packet_no_payload(
    layers: &[LayerInstance],
    registry: &LayerRegistry,
) -> Result<Vec<u8>, String> {
    build_packet(layers, &[], registry)
}

/// Build a packet while applying small, protocol-specific "build glue" to layers.
///
/// Today this focuses on the one annoying real-world detail for UDP: its checksum
/// needs IPv4 src/dst. When building from layers, we allow the UDP layer to carry
/// optional IPs, then fill them from the IPv4 layer when present.
///
/// Contract:
/// - Callers still provide the registry explicitly (no hidden global state).
/// - If the chain contains `Ipv4` and `Udp`, and the `Udp` layer has missing
///   `src_ip/dst_ip`, they will be filled from the `Ipv4` layer.
pub fn build_packet_with_glue(
    layers: &[LayerInstance],
    payload: &[u8],
    registry: &LayerRegistry,
) -> Result<Vec<u8>, String> {
    use crate::packet::layers::{Ipv4, Udp};

    // Find IPv4 src/dst if present.
    let mut ipv4_pair: Option<(crate::Ipv4Addr, crate::Ipv4Addr)> = None;
    for l in layers {
        if l.id == LayerId::Ipv4 {
            if let Some(ip) = l.downcast_ref::<Ipv4>() {
                ipv4_pair = Some((ip.src, ip.dst));
            }
            break;
        }
    }

    // Clone layers only if we actually need to patch anything.
    let Some((src, dst)) = ipv4_pair else {
        return build_packet(layers, payload, registry);
    };

    let mut patched: Vec<LayerInstance> = Vec::new();
    let mut changed = false;
    for l in layers {
        if l.id == LayerId::Udp {
            if let Some(udp) = l.downcast_ref::<Udp>() {
                let mut u = *udp;
                if u.src_ip.is_none() {
                    u.src_ip = Some(src);
                    changed = true;
                }
                if u.dst_ip.is_none() {
                    u.dst_ip = Some(dst);
                    changed = true;
                }
                patched.push(LayerInstance {
                    id: LayerId::Udp,
                    inner: Box::new(u),
                });
                continue;
            }
        }
        // We can't clone arbitrary `Box<dyn Any>` here, so we only support patching
        // the UDP layer and only when the chain contains *exactly* the built-in
        // Ether/Ipv4/Udp triplet.
        if l.id == LayerId::Ether {
            if let Some(eth) = l.downcast_ref::<crate::packet::layers::Ether>() {
                patched.push(LayerInstance {
                    id: LayerId::Ether,
                    inner: Box::new(*eth),
                });
                continue;
            }
        }
        if l.id == LayerId::Ipv4 {
            if let Some(ip) = l.downcast_ref::<Ipv4>() {
                patched.push(LayerInstance {
                    id: LayerId::Ipv4,
                    inner: Box::new(*ip),
                });
                continue;
            }
        }

        // Unknown layer in chain => no glue applied.
        return build_packet(layers, payload, registry);
    }

    if changed {
        build_packet(&patched, payload, registry)
    } else {
        build_packet(layers, payload, registry)
    }
}

/// Wrapper over [`build_packet_with_glue`] that defaults payload to empty.
pub fn build_packet_no_payload_with_glue(
    layers: &[LayerInstance],
    registry: &LayerRegistry,
) -> Result<Vec<u8>, String> {
    build_packet_with_glue(layers, &[], registry)
}

/// Parse a packet into a graph.
///
/// MVP: this is a thin wrapper over `parse_packet` producing a linear chain graph.
///
/// Future direction: tunnel-aware layers can add additional inner chains (VXLAN/GRE/Geneve)
/// to turn this into a real graph.
pub fn parse_packet_graph<'a>(
    data: &'a [u8],
    first: LayerId,
    registry: &LayerRegistry,
) -> Result<PacketGraph<'a>, String> {
    fn build_graph<'a>(
        data: &'a [u8],
        base: usize,
        first: LayerId,
        registry: &LayerRegistry,
        nodes: &mut Vec<LayerInstance>,
        spans: &mut Vec<(usize, usize)>,
        edges: &mut Vec<(usize, usize, EdgeKind)>,
    ) -> Result<(), String> {
        let (local_nodes, local_spans, _payload) = parse_packet_with_spans(data, first, registry)?;
        let start_idx = nodes.len();

        // append nodes + spans in the outer coordinate space
        nodes.extend(local_nodes);
        spans.extend(local_spans.iter().map(|(s, e)| (base + *s, base + *e)));

        // chain edges for this segment
        for i in start_idx..nodes.len().saturating_sub(1) {
            edges.push((i, i + 1, EdgeKind::Encapsulates));
        }

        // scan only nodes we just appended
        let mut i = start_idx;
        while i < nodes.len() {
            // compute payload slice for this layer in its own segment: it's the bytes after its header end
            let (_abs_s, abs_e) = spans[i];
            let local_off = abs_e - base;
            let layer_payload = &data[local_off..];
            let inner_first = registry.tunnel_next(nodes[i].id, &*nodes[i].inner, layer_payload)?;
            if let Some(inner_first) = inner_first {
                if !layer_payload.is_empty() {
                    let inner_first_idx = nodes.len();
                    build_graph(
                        layer_payload,
                        base + local_off,
                        inner_first,
                        registry,
                        nodes,
                        spans,
                        edges,
                    )?;
                    edges.push((i, inner_first_idx, EdgeKind::Tunnels));
                }
            }
            i += 1;
        }

        Ok(())
    }

    let mut nodes = Vec::new();
    let mut spans = Vec::new();
    let mut edges = Vec::new();
    build_graph(data, 0, first, registry, &mut nodes, &mut spans, &mut edges)?;

    // The returned payload remains the "tail" of the outer-most parsing.
    // (Graph edges allow reaching inner payloads.)
    let (_outer_nodes, _outer_spans, payload) = parse_packet_with_spans(data, first, registry)?;
    Ok(PacketGraph::new(nodes, edges, payload))
}
