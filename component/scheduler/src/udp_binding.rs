//! UDP socket binding helpers used by scheduler.

use crate::ntx::core_types::types::ActionDef;

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
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
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
        *resources = serde_json::json!({});
    }
    let obj = resources.as_object_mut().unwrap();
    let bound = obj
        .entry("_bound".to_string())
        .or_insert(serde_json::json!({}));
    if let Some(bobj) = bound.as_object_mut() {
        bobj.insert(
            "udp_owner_id".to_string(),
            serde_json::Value::String(owner_id.to_string()),
        );
    }
}
