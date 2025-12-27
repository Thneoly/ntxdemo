use std::collections::HashMap;

use crate::{PacketRxPayload, Scenario, WaitOnSpec};

pub(crate) fn wait_match(
    on: Option<&WaitOnSpec>,
    action_id: &str,
    task_id: &str,
    p: &PacketRxPayload,
) -> bool {
    let Some(on) = on else {
        return false;
    };
    let m = &on.r#match;
    if m.is_null() {
        return true;
    }
    // 支持 match.action_id / match.task_id
    let mut ok = true;
    if let Some(exp) = m.get("action_id").and_then(|v| v.as_str()) {
        ok &= exp == action_id;
    }
    if let Some(exp) = m.get("task_id").and_then(|v| v.as_str()) {
        ok &= exp == task_id;
    }
    // 支持 match.sock_id / match.len / match.payload_hex
    if let Some(exp) = m
        .get("sock_id")
        .or_else(|| m.get("sock-id"))
        .and_then(|v| v.as_u64())
    {
        ok &= p.sock_id == exp;
    }
    if let Some(exp) = m.get("len").and_then(|v| v.as_u64()) {
        ok &= u64::try_from(p.len)
            .ok()
            .map(|got| got == exp)
            .unwrap_or(false);
    }
    if let Some(exp) = m
        .get("payload_hex")
        .or_else(|| m.get("payload-hex"))
        .and_then(|v| v.as_str())
    {
        let norm = |s: &str| s.trim().trim_start_matches("0x").to_ascii_lowercase();
        ok &= norm(&p.payload_hex) == norm(exp);
    }
    ok
}

pub(crate) fn node_priority(sc: &Scenario, node_id: &str) -> i32 {
    sc.workflows
        .nodes
        .iter()
        .find(|n| n.id == node_id)
        .and_then(|n| n.priority)
        .unwrap_or(0)
}

pub(crate) fn edge_trigger_allows(
    trigger: Option<&serde_json::Value>,
    reason: &str,
    eval_ctx: Option<&serde_json::Value>,
) -> bool {
    let Some(t) = trigger else {
        return true;
    };
    if t.is_null() {
        return true;
    }
    let Some(obj) = t.as_object() else {
        return true;
    };

    // 最小支持：trigger.on / trigger.event / trigger.status
    let mut allow = None::<bool>;
    if let Some(on) = obj.get("on").and_then(|v| v.as_str()) {
        allow = Some(crate::match_reason(on, reason));
    }
    if allow.is_none() {
        if let Some(ev) = obj.get("event").and_then(|v| v.as_str()) {
            allow = Some(crate::match_reason(ev, reason));
        }
    }
    if allow.is_none() {
        if let Some(st) = obj.get("status").and_then(|v| v.as_str()) {
            allow = Some(crate::match_reason(st, reason));
        }
    }

    // 先按 on/event/status 过滤：若显式指定且不匹配，则拒绝
    if matches!(allow, Some(false)) {
        return false;
    }

    // trigger.condition：受限表达式（==/!=/contains + &&/||），基于 eval_ctx 求值
    if let Some(cond) = obj.get("condition").and_then(|v| v.as_str()) {
        let Some(ec) = eval_ctx else {
            println!(
                "[scheduler] warn: edge trigger.condition provided but no eval_ctx, rejecting: {}",
                t
            );
            return false;
        };
        match crate::eval_condition(cond, ec) {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                println!(
                    "[scheduler] warn: eval condition failed, rejecting edge; condition=`{}` err={}",
                    cond, e
                );
                return false;
            }
        }
    }

    true
}

pub(crate) fn find_start_nodes(sc: &Scenario) -> Vec<String> {
    if sc.workflows.nodes.iter().any(|n| n.id == "start") {
        return vec!["start".to_string()];
    }
    // no incoming edges nodes
    let mut has_incoming: HashMap<&str, bool> = HashMap::new();
    for n in &sc.workflows.nodes {
        has_incoming.insert(&n.id, false);
    }
    for n in &sc.workflows.nodes {
        for e in &n.edges {
            has_incoming.insert(&e.to, true);
        }
    }
    sc.workflows
        .nodes
        .iter()
        .filter(|n| !has_incoming.get(n.id.as_str()).copied().unwrap_or(false))
        .map(|n| n.id.clone())
        .collect()
}
