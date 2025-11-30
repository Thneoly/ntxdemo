# Actions-HTTP Quick Reference

## 组件信息

**名称**: scheduler-actions-http  
**版本**: 0.1.0  
**Target**: wasm32-wasip2  
**大小**: ~750KB (release), ~13MB (debug)  
**位置**: `target/wasm32-wasip2/release/scheduler_actions_http.wasm`

## 核心功能

✅ **HTTP/1.1 客户端** - 基于 core-libs Socket API  
✅ **支持方法**: GET, POST, PUT, DELETE, PATCH, etc.  
✅ **Header 管理** - 自定义请求头  
✅ **Body 支持** - 文本和二进制数据  
✅ **响应解析** - Status code, headers, body  
⏳ **DNS** - 当前支持 IP 地址，域名待实现  
❌ **HTTPS** - TLS 支持待实现  
❌ **IP 池绑定** - 源 IP 选择待实现  

## API 接口

### 导出接口

**scheduler:actions-http/http-component@0.1.0**:
```wit
init-component: func() -> result<_, string>;
do-http-action: func(action: action-def) -> result<action-outcome, string>;
release-component: func() -> result<_, string>;
```

**scheduler:actions-http/types@0.1.0**:
```wit
record action-def {
    id: string,
    call: string,  // HTTP method
    with-params: string,  // JSON encoded parameters
    exports: list<export-def>,
}

record action-outcome {
    status: action-status,  // success | failed
    detail: option<string>,
}
```

### 参数格式

**with-params JSON 格式**:
```json
{
  "url": "http://host:port/path",
  "headers": {
    "Header-Name": "Header-Value"
  },
  "body": "request body content"
}
```

## 使用示例

### 示例 1: GET 请求

```rust
let action = ActionDef {
    id: "fetch-api".to_string(),
    call: "GET".to_string(),
    with_params: serde_json::to_string(&json!({
        "url": "http://192.168.1.100:8080/api/data",
        "headers": {
            "Accept": "application/json",
            "User-Agent": "Scheduler-Client"
        }
    }))?,
    exports: vec![],
};

let outcome = component.do_http_action(action)?;
// outcome.status == ActionStatus::Success
// outcome.detail == "GET http://192.168.1.100:8080/api/data status=200 body_len=1234"
```

### 示例 2: POST 请求

```rust
let action = ActionDef {
    id: "submit-data".to_string(),
    call: "POST".to_string(),
    with_params: serde_json::to_string(&json!({
        "url": "http://10.0.0.5:3000/api/submit",
        "headers": {
            "Content-Type": "application/json"
        },
        "body": "{\"key\":\"value\"}"
    }))?,
    exports: vec![],
};

let outcome = component.do_http_action(action)?;
```

### 示例 3: DSL 格式

```yaml
actions:
  - id: http-get
    call: GET
    with:
      url: "http://api.example.com/status"
      headers:
        Authorization: "Bearer {{token}}"
    export:
      - type: variable
        name: response
        scope: step
```

## 响应格式

### 成功响应

```
ActionOutcome {
    status: Success,
    detail: Some("GET http://host/path status=200 body_len=1234")
}
```

### 失败响应

```
ActionOutcome {
    status: Failed,
    detail: Some("GET http://host/path status=404 body=Not Found")
}
```

### 错误响应

```
ActionOutcome {
    status: Failed,
    detail: Some("HTTP request failed: Failed to connect: Connection refused")
}
```

## 限制和注意事项

### 当前限制

1. **仅支持 HTTP** - HTTPS/TLS 尚未实现
2. **DNS 限制** - 仅支持 IP 地址、localhost、0.0.0.0
3. **无 Keep-Alive** - 每次请求建立新连接
4. **同步阻塞** - 不支持异步并发请求
5. **无代理支持** - 直连模式

### URL 要求

✅ 支持: `http://192.168.1.100:8080/path`  
✅ 支持: `http://localhost:3000/api`  
✅ 支持: `http://127.0.0.1/test`  
❌ 不支持: `https://...` (HTTPS)  
❌ 不支持: `http://domain.com` (域名 DNS)  

### Body 限制

- 支持任意文本内容
- 支持二进制数据
- 无大小限制（受内存限制）
- 不支持 multipart/form-data（需手动构建）

## 错误处理

### 常见错误

| 错误信息 | 原因 | 解决方案 |
|---------|------|---------|
| `Failed to parse URL` | URL 格式错误 | 检查 URL 格式 |
| `Cannot resolve hostname` | 域名无法解析 | 使用 IP 地址 |
| `HTTPS not yet supported` | 使用了 HTTPS | 改用 HTTP 或等待 TLS 支持 |
| `Failed to connect` | 无法连接到服务器 | 检查网络和端口 |
| `Failed to send request` | 发送数据失败 | 检查连接状态 |
| `Failed to parse response` | 响应格式错误 | 检查服务器响应 |

## 性能特征

### 延迟

- Socket 创建: <1ms
- TCP 连接: 取决于网络 (通常 1-100ms)
- 数据传输: 取决于带宽和数据大小
- 响应解析: <1ms (小响应)

### 吞吐量

- 单连接: 受网络带宽限制
- 无连接池: 每次请求建立新连接（开销较大）
- 建议: 对频繁请求考虑实现连接池

### 内存使用

- 基础开销: ~10KB per request
- 响应缓冲: response_size * 1.5 (峰值)
- 建议: 对大响应考虑流式处理

## 调试

### 启用详细日志

```rust
// 在 component.rs 中添加日志
eprintln!("DEBUG: Connecting to {}:{}", host, port);
eprintln!("DEBUG: Sending {} bytes", request_bytes.len());
eprintln!("DEBUG: Received {} bytes", response_data.len());
```

### 检查组件接口

```bash
wasm-tools component wit scheduler_actions_http.wasm
```

### 验证请求格式

```bash
# 使用 curl 验证相同的请求
curl -X GET http://192.168.1.100:8080/api/data \
  -H "Accept: application/json" \
  -v
```

## 最佳实践

### 1. URL 模板处理

```rust
// 在发送前解析模板变量
if url.contains("{{") {
    // 跳过或报错
    return Err("Unresolved template variables");
}
```

### 2. Header 设置

```rust
// 始终设置 User-Agent
headers: {
    "User-Agent": "Scheduler-Actions-HTTP/0.1.0"
}

// POST/PUT 需要 Content-Type
headers: {
    "Content-Type": "application/json"
}
```

### 3. 错误处理

```rust
match do_http_action(action) {
    Ok(outcome) if outcome.status == Success => {
        // 处理成功响应
    }
    Ok(outcome) => {
        // HTTP 错误（4xx, 5xx）
        eprintln!("HTTP error: {}", outcome.detail);
    }
    Err(e) => {
        // 网络或系统错误
        eprintln!("System error: {}", e);
    }
}
```

### 4. 超时处理

```rust
// 当前没有内置超时，建议在调用层实现
tokio::time::timeout(
    Duration::from_secs(30),
    do_http_action(action)
)?
```

## 集成指南

### 在 Executor 中集成

```rust
// 1. 加载组件
let component_path = "scheduler_actions_http.wasm";
let component = load_wasm_component(component_path)?;

// 2. 初始化
component.call("init-component", &[])?;

// 3. 执行 action
let result = component.call("do-http-action", &[action_def])?;

// 4. 清理
component.call("release-component", &[])?;
```

### 在 Scheduler 中集成

```rust
// 注册为 action handler
scheduler.register_action_handler("GET", http_component);
scheduler.register_action_handler("POST", http_component);
scheduler.register_action_handler("PUT", http_component);
scheduler.register_action_handler("DELETE", http_component);

// 执行 workbook
scheduler.execute_workbook("workbook.yaml")?;
```

## 相关文档

- **完整文档**: `IMPLEMENTATION_SUMMARY.md`
- **架构设计**: `ARCHITECTURE.md`
- **Core-Libs Socket**: `../core-libs/doc/SOCKET_IP_INTEGRATION.md`
- **测试脚本**: `test_component.sh`

## 联系和支持

- 源码: `/plugins/scheduler/actions-http`
- Issues: 报告到项目 issue tracker
- 更新: 查看 git commit history

---

**最后更新**: 2025-11-30  
**版本**: 0.1.0  
**状态**: ✅ 基本功能完成，增强功能开发中
