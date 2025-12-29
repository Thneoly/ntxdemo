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

    // 支持 match.action_id / match.task_id
    let mut ok = true;
    if let Some(exp) = m.action_id.as_deref() {
        ok &= exp == action_id;
    }
    if let Some(exp) = m.task_id.as_deref() {
        ok &= exp == task_id;
    }
    // 支持 match.sock_id / match.len / match.payload_hex
    if let Some(exp) = m.sock_id {
        ok &= p.sock_id == exp;
    }
    if let Some(exp) = m.len {
        ok &= u64::try_from(p.len)
            .ok()
            .map(|got| got == exp)
            .unwrap_or(false);
    }
    if let Some(exp) = m.payload_hex.as_deref() {
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
    trigger: Option<&crate::TriggerSpec>,
    reason: &str,
    eval_ctx: Option<&serde_json::Value>,
) -> bool {
    let Some(t) = trigger else {
        return true;
    };
    // 先按 on/event/status 过滤：若显式指定且不匹配，则拒绝
    if let Some(when) = t.when.as_ref() {
        let wants = match when {
            crate::TriggerWhen::On { on }
            | crate::TriggerWhen::Event { event: on }
            | crate::TriggerWhen::Status { status: on } => *on,
        };
        let allow = match wants {
            crate::TriggerReason::Success => crate::match_reason("success", reason),
            crate::TriggerReason::Failed => crate::match_reason("failed", reason),
            crate::TriggerReason::Timeout => crate::match_reason("timeout", reason),
            crate::TriggerReason::PacketRx => {
                crate::match_reason(crate::EventKind::PacketRx.as_str(), reason)
            }
            crate::TriggerReason::Other => true,
        };
        if !allow {
            return false;
        }
    }

    // trigger.condition：受限表达式（==/!=/contains + &&/||），基于 eval_ctx 求值
    if let Some(cond) = t.condition.as_deref() {
        let Some(ec) = eval_ctx else {
            println!(
                "[scheduler] warn: edge trigger.condition provided but no eval_ctx, rejecting: {:?}",
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
        .filter(|n| {
            !has_incoming.get(n.id.as_str()).copied().unwrap_or(false)
                && n.kind == crate::NodeKind::Action
        })
        .map(|n| n.id.clone())
        .collect()
}
