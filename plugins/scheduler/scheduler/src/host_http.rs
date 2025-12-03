use anyhow::{Result, anyhow};

use crate::component::scheduler::actions_http::http_component;
use crate::core::dsl::ActionDef;
use crate::http_bridge::{from_wit_outcome, to_wit_action_def};
use crate::{ActionComponent, ActionContext, ActionOutcome};

/// 基于 actions-http WIT 接口的 ActionComponent 实现
pub struct WitHttpActionComponent {
    initialized: bool,
}

impl WitHttpActionComponent {
    pub fn new() -> Self {
        Self { initialized: false }
    }

    fn ensure_initialized(&mut self) -> Result<()> {
        if !self.initialized {
            http_component::init_component()
                .map_err(|e| anyhow!("http-component init failed: {e}"))?;
            self.initialized = true;
        }
        Ok(())
    }
}

impl Default for WitHttpActionComponent {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WitHttpActionComponent {
    fn drop(&mut self) {
        if self.initialized {
            let _ = http_component::release_component();
            self.initialized = false;
        }
    }
}

impl ActionComponent for WitHttpActionComponent {
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
        let outcome = http_component::do_http_action(&wit_action)
            .map_err(|e| anyhow!("http action failed: {e}"))?;
        Ok(from_wit_outcome(outcome))
    }

    fn release(&mut self) -> Result<()> {
        if self.initialized {
            http_component::release_component()
                .map_err(|e| anyhow!("http-component release failed: {e}"))?;
            self.initialized = false;
        }
        Ok(())
    }
}
