//! 条件/表达式求值（供 workflow trigger.condition 使用）
//!
//! 语法（最小实现，保持与原 lib.rs 行为一致）：
//! - `||` / `&&`（忽略引号内的分隔符）
//! - 原子表达式：
//!   - `<path> == <literal>`
//!   - `<path> != <literal>`
//!   - `<path> contains <literal>`
//!   - 或 bare `<path>`（truthy 判定）
//!
//! 注意：这里的实现刻意保持“简单可用”，不尝试实现完整 DSL。

/// 事件 reason 的匹配（允许 success/failure/timeout 等别名，且做轻度归一化）。
pub(crate) fn match_reason(expr: &str, reason: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .to_ascii_lowercase()
            .replace('_', ".")
            .replace('-', ".")
    };
    let a = norm(expr);
    let b = norm(reason);
    // allow common aliases
    match (a.as_str(), b.as_str()) {
        ("packet.rx", "packet.rx") => true,
        ("success", "success") => true,
        ("failed", "failed") => true,
        ("failure", "failed") => true,
        ("timeout", "timeout") => true,
        ("action.success", "success") => true,
        ("action.failed", "failed") => true,
        ("action.timeout", "timeout") => true,
        _ => a == b,
    }
}

pub(crate) fn eval_condition(expr: &str, ctx: &serde_json::Value) -> Result<bool, String> {
    // OR has lower precedence than AND
    let ors = split_outside_quotes(expr, "||");
    if ors.len() > 1 {
        for part in ors {
            if eval_condition(part.trim(), ctx)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }
    let ands = split_outside_quotes(expr, "&&");
    if ands.len() > 1 {
        for part in ands {
            if !eval_condition(part.trim(), ctx)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    eval_atom(expr.trim(), ctx)
}

fn eval_atom(expr: &str, ctx: &serde_json::Value) -> Result<bool, String> {
    // Support: <path> == <lit> | != | contains
    if expr.is_empty() {
        return Ok(true);
    }
    // contains
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "contains") {
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        let lv = resolve_path(ctx, lhs);
        let rv = parse_literal(rhs)?;
        let ls = value_to_string(lv);
        let rs = value_to_string(&rv);
        return Ok(ls.contains(&rs));
    }
    // ==
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "==") {
        let lv = resolve_path(ctx, lhs.trim());
        let rv = parse_literal(rhs.trim())?;
        return Ok(values_equal(lv, &rv));
    }
    // !=
    if let Some((lhs, rhs)) = split_once_outside_quotes(expr, "!=") {
        let lv = resolve_path(ctx, lhs.trim());
        let rv = parse_literal(rhs.trim())?;
        return Ok(!values_equal(lv, &rv));
    }
    // Bare identifier: treat as truthy (exists && not false/null/0/"")
    let v = resolve_path(ctx, expr);
    Ok(is_truthy(v))
}

fn is_truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_i64().map(|x| x != 0).unwrap_or(true),
        serde_json::Value::String(s) => {
            !s.is_empty() && s != "0" && s.to_ascii_lowercase() != "false"
        }
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
    }
}

fn values_equal(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    // Prefer numeric compare when both numeric
    if let (Some(na), Some(nb)) = (a.as_f64(), b.as_f64()) {
        return (na - nb).abs() < f64::EPSILON;
    }
    // Else direct JSON equality (covers bool/null/object/array) OR string normalized
    if a == b {
        return true;
    }
    value_to_string(a) == value_to_string(b)
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn parse_literal(s: &str) -> Result<serde_json::Value, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(serde_json::Value::String(String::new()));
    }
    // quoted string
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        let inner = &s[1..s.len().saturating_sub(1)];
        return Ok(serde_json::Value::String(inner.to_string()));
    }
    // bool/null
    let lc = s.to_ascii_lowercase();
    if lc == "true" {
        return Ok(serde_json::Value::Bool(true));
    }
    if lc == "false" {
        return Ok(serde_json::Value::Bool(false));
    }
    if lc == "null" {
        return Ok(serde_json::Value::Null);
    }
    // number
    if let Ok(i) = s.parse::<i64>() {
        return Ok(serde_json::Value::Number(i.into()));
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Ok(serde_json::Value::Number(n));
        }
    }
    // fallback: bareword string
    Ok(serde_json::Value::String(s.to_string()))
}

fn resolve_path<'a>(ctx: &'a serde_json::Value, path: &str) -> &'a serde_json::Value {
    static NULL: serde_json::Value = serde_json::Value::Null;
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty()).collect();
    let mut cur = ctx;
    for p in parts {
        match cur.get(p) {
            Some(v) => cur = v,
            None => return &NULL,
        }
    }
    cur
}

fn split_once_outside_quotes<'a>(s: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    // For "contains" we expect it to be delimited by whitespace or operators; but keep MVP.
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = s.as_bytes();
    let opb = op.as_bytes();
    let mut i = 0usize;
    while i + opb.len() <= bytes.len() {
        let c = bytes[i] as char;
        if c == '\'' && !in_dq {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '"' && !in_sq {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_sq && !in_dq && &bytes[i..i + opb.len()] == opb {
            let (a, b) = s.split_at(i);
            let b = &b[op.len()..];
            return Some((a, b));
        }
        i += 1;
    }
    None
}

fn split_outside_quotes<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut in_sq = false;
    let mut in_dq = false;
    let bytes = s.as_bytes();
    let sepb = sep.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + sepb.len() <= bytes.len() {
        let c = bytes[i] as char;
        if c == '\'' && !in_dq {
            in_sq = !in_sq;
            i += 1;
            continue;
        }
        if c == '"' && !in_sq {
            in_dq = !in_dq;
            i += 1;
            continue;
        }
        if !in_sq && !in_dq && &bytes[i..i + sepb.len()] == sepb {
            out.push(&s[start..i]);
            i += sepb.len();
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}
