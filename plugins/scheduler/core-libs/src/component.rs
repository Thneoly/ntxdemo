// Component bindings for scheduler-core
#[cfg(target_arch = "wasm32")]
wit_bindgen::generate!({
    world: "scheduler-core",
    path: "wit",
});

#[cfg(target_arch = "wasm32")]
struct SchedulerCoreImpl;

#[cfg(target_arch = "wasm32")]
impl exports::scheduler::core_libs::parser::Guest for SchedulerCoreImpl {
    fn parse_scenario(
        yaml: String,
    ) -> Result<exports::scheduler::core_libs::types::Scenario, String> {
        use crate::dsl::Scenario;
        
        let scenario = Scenario::from_yaml_str(&yaml)
            .map_err(|e| format!("parse error: {}", e))?;

        Ok(exports::scheduler::core_libs::types::Scenario {
            version: scenario.version.clone(),
            name: scenario.name.clone(),
            resources: vec![],
            actions: vec![],
            nodes: vec![],
        })
    }

    fn validate_scenario(
        _scenario: exports::scheduler::core_libs::types::Scenario,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
export!(SchedulerCoreImpl);
