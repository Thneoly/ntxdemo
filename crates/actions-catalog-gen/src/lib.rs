use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::path::Path;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

// Host-side component bindings.
wasmtime::component::bindgen!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
        "../../wit/eventbus",
        "../../wit/types",
        "../../wit/actions-executor",
    ],
    exports: { default: async },
    imports: { default: trappable },
});

#[derive(Default)]
struct HostState {
    table: ResourceTable,
    wasi: WasiCtx,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

// Minimal stubs for event-bus import (catalog APIs should be pure metadata, but the import must exist).
impl crate::ntx::scenario_eventbus::event_bus::Host for HostState {
    fn publish(
        &mut self,
        _event: crate::ntx::scenario_eventbus::event_bus::Event,
    ) -> std::result::Result<std::result::Result<(), String>, anyhow::Error> {
        Ok(Ok(()))
    }

    fn subscribe(
        &mut self,
        _topic: String,
    ) -> std::result::Result<std::result::Result<String, String>, anyhow::Error> {
        Ok(Ok("subscription".to_string()))
    }

    fn unsubscribe(
        &mut self,
        _subscription_id: String,
    ) -> std::result::Result<std::result::Result<(), String>, anyhow::Error> {
        Ok(Ok(()))
    }

    fn poll_events(
        &mut self,
        _subscription_id: String,
        _max: u32,
    ) -> std::result::Result<
        std::result::Result<Vec<crate::ntx::scenario_eventbus::event_bus::Event>, String>,
        anyhow::Error,
    > {
        Ok(Ok(Vec::new()))
    }

    fn wait_events(
        &mut self,
        _subscription_id: String,
        _max: u32,
        _timeout_ms: u32,
    ) -> std::result::Result<
        std::result::Result<Vec<crate::ntx::scenario_eventbus::event_bus::Event>, String>,
        anyhow::Error,
    > {
        Ok(Ok(Vec::new()))
    }
}

// --------- JSON output model (stable for frontend) ---------

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct Catalog {
    pub schema_version: u32,
    pub executor: ExecutorInfo,
    pub actions: Vec<ActionEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ExecutorInfo {
    pub component_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ActionEntry {
    pub summary: ActionSummaryJson,
    pub spec: ActionSpecJson,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ActionSummaryJson {
    pub id: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ActionSpecJson {
    pub id: String,
    pub title: String,
    pub description: String,
    pub params_schema_json: String,
    pub default_params_json: String,
    pub capabilities: Vec<ActionCapabilityJson>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct ActionCapabilityJson {
    pub debug: String,
}

pub fn load_catalog_from_component_sync(component_path: &Path) -> Result<Catalog> {
    let rt = tokio::runtime::Runtime::new().context("create tokio runtime")?;
    rt.block_on(load_catalog_from_component(component_path))
}

pub async fn load_catalog_from_component(component_path: &Path) -> Result<Catalog> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model_async(true);
    config.async_support(true);
    let engine = Engine::new(&config)?;

    let component = Component::from_file(&engine, component_path)
        .with_context(|| format!("load component {}", component_path.display()))?;

    let mut linker = Linker::<HostState>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    crate::ntx::scenario_eventbus::event_bus::add_to_linker::<_, HasSelf<HostState>>(
        &mut linker,
        |s: &mut HostState| s,
    )
    .context("wire event-bus import")?;

    let wasi = WasiCtxBuilder::new()
        .inherit_stdout()
        .inherit_stderr()
        .build();
    let mut store = Store::new(
        &engine,
        HostState {
            table: ResourceTable::new(),
            wasi,
        },
    );

    let executor = ActionExecutorComponent::instantiate_async(&mut store, &component, &linker)
        .await
        .context("instantiate actions-executor component")?;

    let schema_version = executor
        .ntx_scenario_actions_executor_action_component()
        .call_schema_version(&mut store)
        .await
        .context("schema_version")?;

    let list = executor
        .ntx_scenario_actions_executor_action_component()
        .call_list_actions(&mut store)
        .await
        .context("list_actions")?;

    let mut actions = Vec::with_capacity(list.len());
    for summary in list {
        let spec = executor
            .ntx_scenario_actions_executor_action_component()
            .call_describe_action(&mut store, &summary.id)
            .await
            .map_err(|e| anyhow!("describe_action({}): {e}", summary.id))?;
        let spec = spec.map_err(|e| anyhow!("describe_action({}): {e}", summary.id))?;

        actions.push(ActionEntry {
            summary: ActionSummaryJson {
                id: summary.id.clone(),
                title: summary.title.clone(),
                description: summary.description.clone().unwrap_or_default(),
            },
            spec: ActionSpecJson {
                id: spec.id.clone(),
                title: spec.title.clone(),
                description: spec.description.clone().unwrap_or_default(),
                params_schema_json: spec.input_schema_json,
                default_params_json: spec.defaults_json.unwrap_or_default(),
                capabilities: spec
                    .capabilities
                    .into_iter()
                    .map(|c| ActionCapabilityJson {
                        debug: format!("{c:?}"),
                    })
                    .collect(),
            },
        });
    }

    actions.sort_by(|a, b| a.summary.id.cmp(&b.summary.id));

    Ok(Catalog {
        schema_version,
        executor: ExecutorInfo {
            component_path: component_path.display().to_string(),
        },
        actions,
    })
}
