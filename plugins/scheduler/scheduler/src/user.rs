use anyhow::{Context, Result};
use indexmap::IndexMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use crate::TemplateContext;
use scheduler_core::dsl::{ActionDef, ActionsSection, WorkflowNodeType, WorkflowSection};
use scheduler_executor::{ActionComponent, ActionContext};

/// 用户上下文
///
/// 包含用户的身份信息和资源分配
#[derive(Debug, Clone)]
pub struct UserContext {
    pub id: usize,
    pub tenant_id: String,
    pub allocated_ip: Option<IpAddr>,
    pub created_at: Instant,
}

impl UserContext {
    pub fn new(id: usize, tenant_id: String, allocated_ip: Option<IpAddr>) -> Self {
        Self {
            id,
            tenant_id,
            allocated_ip,
            created_at: Instant::now(),
        }
    }

    pub fn new_with_id(id: usize, tenant_id: String, allocated_ip: Option<IpAddr>) -> Self {
        Self::new(id, tenant_id, allocated_ip)
    }
}

/// 执行跟踪记录
#[derive(Debug, Clone)]
pub struct ExecutionTrace {
    pub user_id: usize,
    pub iteration: usize,
    pub action_id: String,
    pub status: String,
    pub detail: String,
    pub duration_ms: u64,
}

/// 用户执行器
///
/// 负责执行用户的工作流，支持：
/// - 多次迭代执行
/// - 变量替换（{{user.allocated_ip}} 等）
/// - Think time 控制
/// - 执行跟踪
pub struct UserExecutor {
    context: UserContext,
    workflow: WorkflowSection,
    actions: ActionsSection,
    iterations: usize,
    think_time: Duration,
    template: TemplateContext,
}

impl UserExecutor {
    /// 创建新的用户执行器
    ///
    /// # Arguments
    /// * `context` - 用户上下文
    /// * `workflow` - 工作流定义
    /// * `actions` - 动作定义列表
    /// * `iterations` - 迭代次数（0 = 无限循环）
    /// * `think_time` - 每次迭代之间的等待时间
    pub fn new(
        context: UserContext,
        workflow: WorkflowSection,
        actions: ActionsSection,
        iterations: usize,
        think_time: Duration,
        template: TemplateContext,
    ) -> Self {
        Self {
            context,
            workflow,
            actions,
            iterations,
            think_time,
            template,
        }
    }

    /// 执行用户的所有迭代
    ///
    /// # Arguments
    /// * `component` - ActionComponent 实现，用于执行具体的动作
    ///
    /// # Returns
    /// 所有迭代的执行跟踪列表
    pub fn run<C: ActionComponent>(&mut self, component: &mut C) -> Result<Vec<ExecutionTrace>> {
        let mut all_traces = Vec::new();

        let actual_iterations = if self.iterations == 0 {
            usize::MAX // 无限循环（实际上会被外部中断）
        } else {
            self.iterations
        };

        let iterations_label = if self.iterations == 0 {
            "∞".to_string()
        } else {
            self.iterations.to_string()
        };

        for iteration in 0..actual_iterations {
            if iteration > 0 {
                std::thread::sleep(self.think_time);
            }

            println!(
                "[User-{}] Starting iteration {}/{}",
                self.context.id,
                iteration + 1,
                iterations_label.as_str()
            );

            // 执行一次完整的 workflow
            let mut iteration_traces =
                self.execute_workflow(component, iteration)
                    .with_context(|| {
                        format!(
                            "User {} iteration {} failed",
                            self.context.id,
                            iteration + 1
                        )
                    })?;

            all_traces.append(&mut iteration_traces);
        }

        println!(
            "[User-{}] Completed {} iterations",
            self.context.id,
            iterations_label.as_str()
        );

        Ok(all_traces)
    }

    /// 执行一次完整的 workflow
    fn execute_workflow<C: ActionComponent>(
        &mut self,
        component: &mut C,
        iteration: usize,
    ) -> Result<Vec<ExecutionTrace>> {
        let mut traces = Vec::new();
        let mut current_node = String::from("start");
        let mut execution_context = IndexMap::new();

        // 创建一个临时的 WbsTree 用于 ActionContext
        let temp_wbs = scheduler_core::wbs::WbsTree::new_empty();

        // 注入用户变量到上下文
        execution_context.insert("user.id".to_string(), self.context.id.to_string());
        execution_context.insert("tenant.id".to_string(), self.context.tenant_id.clone());
        if let Some(ip) = self.context.allocated_ip {
            execution_context.insert("user.allocated_ip".to_string(), ip.to_string());
        }

        loop {
            // 查找当前节点
            let node = self
                .workflow
                .nodes
                .iter()
                .find(|n| n.id == current_node)
                .with_context(|| format!("Node '{}' not found", current_node))?;

            match node.node_type {
                WorkflowNodeType::Action => {
                    // 获取动作定义
                    let action_id = node
                        .action
                        .as_ref()
                        .with_context(|| format!("Node '{}' has no action", current_node))?;

                    let action = self
                        .actions
                        .actions
                        .iter()
                        .find(|a| &a.id == action_id)
                        .with_context(|| format!("Action '{}' not found", action_id))?;

                    // 替换变量
                    let resolved_action = self.resolve_variables(action, &execution_context)?;

                    // 执行动作
                    let start = Instant::now();
                    let mut action_ctx = ActionContext::new(&temp_wbs);
                    let outcome = component
                        .do_action(&resolved_action, &mut action_ctx)
                        .with_context(|| format!("Action '{}' execution failed", action_id))?;
                    let duration = start.elapsed();

                    // 记录跟踪
                    traces.push(ExecutionTrace {
                        user_id: self.context.id,
                        iteration,
                        action_id: action_id.clone(),
                        status: format!("{:?}", outcome.status),
                        detail: outcome.detail.unwrap_or_default(),
                        duration_ms: duration.as_millis() as u64,
                    });

                    // TODO: 从 ActionContext 获取输出并更新上下文
                    // 当前简化实现：不保存动作输出

                    // 选择下一个节点
                    current_node = self.select_next_node(node, &execution_context)?;
                }
                WorkflowNodeType::End => {
                    // 到达终点
                    break;
                }
            }
        }

        Ok(traces)
    }

    /// 替换动作中的变量
    ///
    /// 支持的变量：
    /// - {{user.id}} - 用户 ID
    /// - {{user.allocated_ip}} - 用户分配的 IP
    /// - {{tenant.id}} - 租户 ID
    /// - {{action.property}} - 之前动作的输出
    fn resolve_variables(
        &self,
        action: &ActionDef,
        context: &IndexMap<String, String>,
    ) -> Result<ActionDef> {
        let merged_ctx = self.template.merged(context);
        Ok(merged_ctx.render_action(action))
    }

    /// 根据条件选择下一个节点
    fn select_next_node(
        &self,
        node: &scheduler_core::dsl::WorkflowNode,
        context: &IndexMap<String, String>,
    ) -> Result<String> {
        let merged_ctx = self.template.merged(context);
        for edge in &node.edges {
            // 如果没有条件，直接选择
            if edge.trigger.is_none() {
                return Ok(edge.to.clone());
            }

            // 评估条件（简化版：只支持 "true" 或简单的相等判断）
            if let Some(trigger) = &edge.trigger {
                if let Some(condition) = &trigger.condition {
                    if condition == "true" || self.evaluate_condition(condition, &merged_ctx) {
                        return Ok(edge.to.clone());
                    }
                }
            }
        }

        anyhow::bail!("No matching edge found for node '{}'", node.id)
    }

    /// 简单的条件评估
    ///
    /// 支持格式：{{variable}} == value
    fn evaluate_condition(&self, condition: &str, context: &TemplateContext) -> bool {
        // 简化实现：只支持 "true" 和基本的相等比较
        if condition == "true" {
            return true;
        }

        // 替换变量后评估
        let resolved = context.render_str(condition);

        // 简单的相等判断（如 "200 == 200"）
        if let Some(pos) = resolved.find("==") {
            let left = resolved[..pos].trim();
            let right = resolved[pos + 2..].trim();
            return left == right;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scheduler_core::dsl::{TriggerDef, WorkflowEdge, WorkflowNode};
    use scheduler_executor::{ActionOutcome, ActionStatus};
    use serde_yaml::Value;

    // 简单的测试 ActionComponent
    struct TestComponent;

    impl ActionComponent for TestComponent {
        fn init(&mut self) -> Result<()> {
            Ok(())
        }

        fn do_action(
            &mut self,
            _action: &ActionDef,
            _ctx: &mut ActionContext<'_>,
        ) -> Result<ActionOutcome> {
            Ok(ActionOutcome {
                status: ActionStatus::Success,
                detail: Some("Executed".to_string()),
            })
        }

        fn release(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_user_context_creation() {
        let ctx = UserContext::new(1, "tenant-a".to_string(), Some("10.0.1.1".parse().unwrap()));

        assert_eq!(ctx.id, 1);
        assert_eq!(ctx.tenant_id, "tenant-a");
        assert!(ctx.allocated_ip.is_some());
    }

    #[test]
    fn test_variable_resolution() {
        let action = ActionDef {
            id: "test".to_string(),
            call: "get".to_string(),
            with: {
                let mut map = IndexMap::new();
                map.insert(
                    "url".to_string(),
                    Value::String("http://{{user.allocated_ip}}:8080".to_string()),
                );
                map
            },
            export: vec![],
        };

        let context =
            UserContext::new(1, "tenant-a".to_string(), Some("10.0.1.1".parse().unwrap()));
        let workflow = WorkflowSection { nodes: vec![] };
        let actions = ActionsSection { actions: vec![] };

        let executor = UserExecutor::new(
            context,
            workflow,
            actions,
            1,
            Duration::from_secs(0),
            TemplateContext::new(),
        );

        let mut exec_ctx = IndexMap::new();
        exec_ctx.insert("user.allocated_ip".to_string(), "10.0.1.1".to_string());

        let resolved = executor.resolve_variables(&action, &exec_ctx).unwrap();
        let url = resolved.with.get("url").unwrap().as_str().unwrap();

        assert_eq!(url, "http://10.0.1.1:8080");
    }
}
