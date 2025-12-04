# IP 变量替换问题修复总结

## 问题描述

在 scheduler 组件中，用户分配的 IP 地址（`{{user.allocated_ip}}`）没有被正确替换到 HTTP 请求中。

## 根本原因

经过调试发现了两个问题：

### 1. Workbook 中的循环引用问题

在测试场景 `http_scenario.yaml` 中：
```yaml
resources:
  - id: resource
    type: http_endpoint
    properties:
      ip: "{{resource.ip}}"    # 循环引用！
      port: "{{resource.port}}" # 循环引用！
```

这导致变量永远无法解析，因为 `resource.ip` 的值就是 `"{{resource.ip}}"`。

### 2. 缺少递归变量解析

当 workbook 的 properties 包含模板变量引用时，系统没有递归解析这些引用，导致：
- 简单引用无法工作：`url: "{{base.protocol}}://{{base.host}}"`
- 多层引用更是无法工作：`endpoint: "{{base.url}}/api"`

## 解决方案

### 方案 1: 修复测试 YAML 文件（快速修复）

将循环引用的变量改为实际值：
```yaml
resources:
  - id: resource
    type: http_endpoint
    properties:
      ip: "httpbin.org"    # 实际值
      port: "443"          # 实际值
```

### 方案 2: 添加递归变量解析功能（完整解决方案）

在 `template.rs` 中添加 `resolve_recursively()` 方法：

```rust
fn resolve_recursively(&mut self) {
    const MAX_ITERATIONS: usize = 10;
    
    for _ in 0..MAX_ITERATIONS {
        let mut any_changes = false;
        let mut resolved = IndexMap::new();
        
        for (key, value) in &self.vars {
            let new_value = self.render_str(value);
            if &new_value != value {
                any_changes = true;
            }
            resolved.insert(key.clone(), new_value);
        }
        
        self.vars = resolved;
        
        if !any_changes {
            break;
        }
    }
}
```

在 `from_workbook()` 中调用：
```rust
pub fn from_workbook(workbook: &Workbook) -> Self {
    let mut ctx = TemplateContext::new();
    
    // First pass: collect all variables
    for (resource_id, resource) in &workbook.resources {
        for (prop, value) in &resource.spec.properties {
            if let Some(rendered) = value_to_string(value) {
                ctx.vars.insert(format!("{}.{}", resource_id, prop), rendered);
            }
        }
    }
    
    // Second pass: resolve recursively
    ctx.resolve_recursively();
    
    ctx
}
```

## 验证结果

### 1. IP 变量正确替换

运行调试输出显示：
```
[User-1] Execution context: {"user.id": "1", "tenant.id": "default-tenant", "user.allocated_ip": "10.0.1.0"}
[User-1] Original action.with: {"url": String("https://{{resource.ip}}:{{resource.port}}/get"), "bind_ip": String("{{user.allocated_ip}}")}
[User-1] Resolved action.with: {"url": String("https://httpbin.org:443/get"), "bind_ip": String("10.0.1.0")}
```

✅ `{{user.allocated_ip}}` 正确替换为 `10.0.1.0`
✅ `{{resource.ip}}` 正确替换为 `httpbin.org`
✅ `{{resource.port}}` 正确替换为 `443`

### 2. 递归变量解析工作正常

测试场景 `http_recursive_vars.yaml`：
```yaml
resources:
  - id: base
    properties:
      protocol: "https"
      host: "httpbin.org"
      port: "443"
      url: "{{base.protocol}}://{{base.host}}:{{base.port}}"
  
  - id: final
    properties:
      endpoint: "{{base.url}}/get"
```

解析结果：
```
[Template] Initial: base.url = {{base.protocol}}://{{base.host}}:{{base.port}}
[Template] Initial: final.endpoint = {{base.url}}/get
[Template] After resolution:
[Template]   base.url = https://httpbin.org:443
[Template]   final.endpoint = https://httpbin.org:443/get
```

✅ 多层变量引用正确解析

### 3. 循环引用保护

测试场景 `http_circular_ref.yaml` 包含循环引用：
```yaml
properties:
  var_a: "{{test.var_b}}"
  var_b: "{{test.var_a}}"
```

✅ 系统正常运行，没有无限循环或崩溃

### 4. 单元测试通过

```
running 5 tests
test template::tests::test_multiple_references_in_one_string ... ok
test template::tests::test_circular_reference_handling ... ok
test template::tests::test_simple_variable_substitution ... ok
test template::tests::test_nested_recursive_resolution ... ok
test template::tests::test_recursive_variable_resolution ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured
```

## 修改的文件

1. **scheduler/src/template.rs**
   - 添加 `resolve_recursively()` 方法
   - 修改 `from_workbook()` 调用递归解析
   - 添加完整的单元测试

2. **scheduler/src/user.rs**
   - 添加（后移除）调试输出以验证变量注入
   - 确认变量替换逻辑正常工作

3. **res/http_scenario.yaml**
   - 修复循环引用问题
   - 将 `{{resource.ip}}` 改为实际值 `httpbin.org`
   - 将 `{{resource.port}}` 改为实际值 `443`

4. **新增测试场景**
   - `res/http_recursive_vars.yaml` - 展示递归变量解析
   - `res/http_circular_ref.yaml` - 测试循环引用处理

5. **新增文档**
   - `docs/RECURSIVE_VARIABLE_RESOLUTION.md` - 功能文档

## 功能亮点

1. **自动递归解析**: workbook 变量可以引用其他变量，系统自动解析
2. **循环引用保护**: 最多 10 次迭代，防止无限循环
3. **性能优化**: 解析在初始化时进行一次，不影响运行时性能
4. **向后兼容**: 不影响现有功能，纯增强
5. **完整测试**: 包含 5 个单元测试覆盖各种场景

## 使用示例

```yaml
workbook:
  resources:
    - id: base
      properties:
        protocol: "https"
        host: "api.example.com"
        port: "443"
        base_url: "{{base.protocol}}://{{base.host}}:{{base.port}}"
    
    - id: endpoints
      properties:
        users: "{{base.base_url}}/users"
        posts: "{{base.base_url}}/posts"

actions:
  actions:
    - id: get-user
      call: get
      with:
        url: "{{endpoints.users}}/{{user.id}}"
        bind_ip: "{{user.allocated_ip}}"
```

这将自动解析为：
- `base.base_url` = `https://api.example.com:443`
- `endpoints.users` = `https://api.example.com:443/users`
- 最终 URL = `https://api.example.com:443/users/1` (假设 user.id = 1)
- `bind_ip` = 用户分配的实际 IP

## 结论

问题已完全解决！IP 变量现在可以正确替换，并且系统支持强大的递归变量解析功能，使配置更加灵活和可维护。
