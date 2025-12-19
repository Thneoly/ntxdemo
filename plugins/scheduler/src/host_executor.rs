use anyhow::{Result, anyhow};

use crate::component::scheduler::actions_executor::action_component;
use crate::core::dsl::ActionDef;
use crate::wit_bridge::{from_wit_outcome, to_wit_action_def};
use crate::{ActionComponent, ActionContext, ActionOutcome};

/// 基于 actions-executor WIT 接口的 ActionComponent 实现
pub struct WitActionExecutorComponent {
    initialized: bool,
}

impl WitActionExecutorComponent {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    fn ensure_initialized(&mut self) -> Result<()> {
        if !self.initialized {
            action_component::init_component()
                .map_err(|e| anyhow!("action-component init failed: {e}"))?;
            self.initialized = true;
        }
        Ok(())
    }
}

impl Default for WitActionExecutorComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WitActionExecutorComponent {
    fn drop(&mut self) {
        if self.initialized {
            let _ = action_component::release_component();
            self.initialized = false;
        }
    }
}

impl ActionComponent for WitActionExecutorComponent {
    fn init(&mut self) -> Result<()> {
        self.ensure_initialized()
    }

    fn do_action(
        &mut self,
        action: &ActionDef,
        _ctx: &mut ActionContext<'_>,
    ) -> Result<ActionOutcome> {
        self.ensure_initialized()?;
        let wit_action = to_wit_action_def(action)?;
        let outcome = action_component::execute_action(&wit_action)
            .map_err(|e| anyhow!("action execution failed: {e}"))?;
        Ok(from_wit_outcome(outcome))
    }

    fn release(&mut self) -> Result<()> {
        if self.initialized {
            action_component::release_component()
                .map_err(|e| anyhow!("action-component release failed: {e}"))?;
            self.initialized = false;
        }
        Ok(())
    }
}
