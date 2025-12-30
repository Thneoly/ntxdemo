//! UDP socket binding helpers used by scheduler.

use crate::ntx::core_types::types::ActionDef;
use crate::ntx::host::{resources, types, udp_socket_control};

/// Inject `socket_id` into an action's JSON params if missing.
///
/// Socket id is expected to be already bound into `runtime.users[user_id].resources`.
pub fn inject_udp_socket_id(user_id: &str, def: &mut ActionDef) -> Result<(), String> {
    let sock_id = {
        let rt = crate::RUNTIME
            .lock()
            .map_err(|_| "lock runtime".to_string())?;
        let u = rt
            .users
            .get(user_id)
            .ok_or_else(|| format!("user not found: {}", user_id))?;
        get_bound_udp_socket_id(&u.resources).ok_or_else(|| "udp socket not bound".to_string())?
    };

    let mut v: serde_json::Value =
        serde_json::from_str(&def.params).map_err(|e| format!("decode def.params: {e}"))?;

    if let Some(obj) = v.as_object_mut() {
        obj.entry("socket_id".to_string())
            .or_insert(serde_json::Value::Number(sock_id.into()));
    }

    def.params = serde_json::to_string(&v).map_err(|e| format!("encode def.params: {e}"))?;
    Ok(())
}

pub fn get_bound_udp_socket_id(resources: &serde_json::Value) -> Option<u64> {
    resources
        .get("_bound")
        .and_then(|b| b.get("udp_socket_id"))
        .and_then(|v| v.as_u64())
}

pub fn get_bound_udp_owner_id(resources: &serde_json::Value) -> Option<String> {
    resources
        .get("_bound")
        .and_then(|b| b.get("udp_owner_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn set_bound_udp_socket_id(resources: &mut serde_json::Value, sock_id: u64) {
    if !resources.is_object() {
        // NOTE: runtime resources are stored as JSON for flexible templating/interop.
        // This is internal state (not an event payload schema), so we keep `json!({})`.
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
        // NOTE: internal runtime key-space; not an event payload.
        .or_insert(serde_json::json!({}));
    if let Some(bobj) = bound.as_object_mut() {
        bobj.insert(
            "udp_socket_id".to_string(),
            serde_json::Value::Number(sock_id.into()),
        );
    }
}

pub fn set_bound_udp_owner_id(resources: &mut serde_json::Value, owner_id: &str) {
    if !resources.is_object() {
        // NOTE: runtime resources are stored as JSON for flexible templating/interop.
        // This is internal state (not an event payload schema), so we keep `json!({})`.
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
        // NOTE: internal runtime key-space; not an event payload.
        .or_insert(serde_json::json!({}));
    if let Some(bobj) = bound.as_object_mut() {
        bobj.insert(
            "udp_owner_id".to_string(),
            serde_json::Value::String(owner_id.to_string()),
        );
    }
}

/// Ensure a UDP socket is created+bound for `user_id` and record the binding in runtime.
///
/// This is used by dispatch when an action call is UDP-related.
pub fn ensure_udp_socket_for_user(
    _ctx: &crate::SchedulerContext,
    sc: &crate::scenario::scenario_types::Scenario,
    user_id: &str,
) -> Result<(), String> {
    // already bound?
    if let Ok(rt) = crate::RUNTIME.lock() {
        if let Some(u) = rt.users.get(user_id) {
            if get_bound_udp_socket_id(&u.resources).is_some() {
                return Ok(());
            }
        }
    }

    // pick first udp-endpoint resource
    let res =
        sc.workbook
            .resources
            .iter()
            .find(|r| r.r#type == crate::scenario::scenario_types::ResourceType::UdpEndpoint)
            .or_else(|| {
                sc.workbook.resources.iter().find(|r| {
                    r.r#type == crate::scenario::scenario_types::ResourceType::UdpEndpoint
                })
            })
            .ok_or_else(|| "no udp-endpoint resource in workbook.resources".to_string())?;

    let p = &res.properties;
    let peer_ip = parse_ipv4(p, &["peer_ip", "peer-ip", "peer_ipv4", "peer-ipv4"])
        .ok_or_else(|| "missing peer_ip".to_string())?;
    let peer_mac = parse_mac(
        p,
        &["peer_mac", "peer-mac", "peer_mac_addr", "peer-mac-addr"],
    );
    let peer_port =
        parse_u16(p, &["peer_port", "peer-port"]).ok_or_else(|| "missing peer_port".to_string())?;

    let ttl = p
        .get("ttl")
        .and_then(|v| v.as_u64())
        .and_then(|v| u8::try_from(v).ok());

    // create + bind
    let sock = udp_socket_control::create(user_id)
        .map_err(|e| format!("udp_socket_control.create: {:?}", e))?;

    // pool name: prefer scenario.user_resources.ip_binding.pool_id; then resource.properties.pool; else "default"
    let pool = sc
        .user_resources
        .ip_binding
        .as_ref()
        .and_then(|b| b.pool_id.clone())
        .or_else(|| {
            p.get("pool")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "default".to_string());

    let ident = resources::acquire_udp_identity(&pool, &sock.owner)
        .map_err(|e| format!("resources.acquire_udp_identity(pool={pool}): {:?}", e))?;

    // peer mac may be resolved by host (best-effort) if not configured
    let peer_mac = match peer_mac {
        Some(m) => m,
        None => {
            let mac = resources::resolve_peer_mac(to_wit_ipv4(peer_ip)).map_err(|e| {
                format!("resources.resolve_peer_mac(peer_ip={:?}): {:?}", peer_ip, e)
            })?;
            [mac.a, mac.b, mac.c, mac.d, mac.e, mac.f]
        }
    };

    let bind = udp_socket_control::UdpBind {
        local_ipv4: ident.local_ipv4,
        local_mac: ident.local_mac,
        local_udp_port: ident.local_udp_port,
        peer_ipv4: to_wit_ipv4(peer_ip),
        peer_port,
        peer_mac: to_wit_mac(peer_mac),
        ttl,
    };

    udp_socket_control::bind(sock.sock, bind)
        .map_err(|e| format!("udp_socket_control.bind: {:?}", e))?;

    // store binding into runtime resources
    if let Ok(mut rt) = crate::RUNTIME.lock() {
        if let Some(u) = rt.users.get_mut(user_id) {
            set_bound_udp_socket_id(&mut u.resources, sock.sock);
            set_bound_udp_owner_id(&mut u.resources, &sock.owner);
        }
    }

    // best-effort: also publish a small event for observability
    let _ = crate::ntx::scenario_eventbus::event_bus::publish(
        &crate::ntx::scenario_eventbus::event_bus::Event {
            id: format!(
                "sock-{}",
                crate::EVENT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            kind: crate::EventKind::SchedulerResourceBound
                .as_str()
                .to_string(),
            user_id: Some(user_id.to_string()),
            task_id: None,
            action_id: None,
            payload: serde_json::to_string(&crate::ResourceBoundPayload {
                resource: res.id.clone(),
                sock_id: sock.sock,
            })
            .unwrap_or_else(|_| "{}".to_string()),
            correlation_id: None,
            timestamp_ms: crate::now_ms(),
        },
    );

    Ok(())
}

fn parse_ipv4(props: &serde_json::Value, keys: &[&str]) -> Option<[u8; 4]> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(s) = v.as_str() {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 4 {
            return None;
        }
        let mut out = [0u8; 4];
        for (i, p) in parts.iter().enumerate() {
            out[i] = p.parse::<u8>().ok()?;
        }
        return Some(out);
    }
    if let Some(arr) = v.as_array() {
        if arr.len() != 4 {
            return None;
        }
        let mut out = [0u8; 4];
        for i in 0..4 {
            out[i] = arr[i].as_u64().and_then(|n| u8::try_from(n).ok())?;
        }
        return Some(out);
    }
    None
}

fn parse_mac(props: &serde_json::Value, keys: &[&str]) -> Option<[u8; 6]> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(s) = v.as_str() {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 6 {
            return None;
        }
        let mut out = [0u8; 6];
        for i in 0..6 {
            out[i] = u8::from_str_radix(parts[i], 16).ok()?;
        }
        return Some(out);
    }
    if let Some(arr) = v.as_array() {
        if arr.len() != 6 {
            return None;
        }
        let mut out = [0u8; 6];
        for i in 0..6 {
            out[i] = arr[i].as_u64().and_then(|n| u8::try_from(n).ok())?;
        }
        return Some(out);
    }
    None
}

fn parse_u16(props: &serde_json::Value, keys: &[&str]) -> Option<u16> {
    let v = keys.iter().find_map(|k| props.get(*k))?;
    if let Some(n) = v.as_u64() {
        return u16::try_from(n).ok();
    }
    if let Some(s) = v.as_str() {
        return s.parse::<u16>().ok();
    }
    None
}

fn to_wit_ipv4(ip: [u8; 4]) -> types::Ipv4Addr {
    types::Ipv4Addr {
        a: ip[0],
        b: ip[1],
        c: ip[2],
        d: ip[3],
    }
}

fn to_wit_mac(mac: [u8; 6]) -> types::MacAddr {
    types::MacAddr {
        a: mac[0],
        b: mac[1],
        c: mac[2],
        d: mac[3],
        e: mac[4],
        f: mac[5],
    }
}
