use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::path::{Path, PathBuf};
use wasmtime::component::HasSelf;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "ntx:scenario-actions-executor/action-executor-component@0.1.0",
    path: [
        "../../wit/eventbus",
        "../../wit/types",
        "../../wit/actions-executor",
    ],
    // The actions-executor component is async (WASIp2) so we generate async exports.
    exports: { default: async },
    // Its imports (event-bus, etc) are synchronous host callbacks.
    // We implement only the minimal required import traits.
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

// --------- Host imports (minimal stubs) ---------

// The actions-executor world imports the event-bus interface. For catalog generation we
// don't expect the guest to publish events, but we must satisfy the import at link time.
// Implement as no-ops.
impl crate::ntx::scenario_eventbus::event_bus::Host for HostState {
    fn publish(
        &mut self,
        _event: crate::ntx::scenario_eventbus::event_bus::Event,
    ) -> Result<std::result::Result<(), String>> {
        Ok(Ok(()))
    }

    fn subscribe(&mut self, _topic: String) -> Result<std::result::Result<String, String>> {
        Ok(Ok("subscription".to_string()))
    }

    fn unsubscribe(&mut self, _subscription_id: String) -> Result<std::result::Result<(), String>> {
        Ok(Ok(()))
    }

    fn poll_events(
        &mut self,
        _subscription_id: String,
        _max: u32,
    ) -> Result<std::result::Result<Vec<crate::ntx::scenario_eventbus::event_bus::Event>, String>>
    {
        Ok(Ok(Vec::new()))
    }

    fn wait_events(
        &mut self,
        _subscription_id: String,
        _max: u32,
        _timeout_ms: u32,
    ) -> Result<std::result::Result<Vec<crate::ntx::scenario_eventbus::event_bus::Event>, String>>
    {
        Ok(Ok(Vec::new()))
    }
}

// --------- JSON output model (stable for frontend) ---------

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct Catalog {
    schema_version: u32,
    executor: ExecutorInfo,
    actions: Vec<ActionEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ExecutorInfo {
    component_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ActionEntry {
    summary: ActionSummaryJson,
    spec: ActionSpecJson,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ActionSummaryJson {
    id: String,
    title: String,
    description: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ActionSpecJson {
    id: String,
    title: String,
    description: String,
    params_schema_json: String,
    default_params_json: String,
    capabilities: Vec<ActionCapabilityJson>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
struct ActionCapabilityJson {
    // Keep this tool decoupled from WIT field renames by preserving a textual
    // representation. The frontend is expected to treat capabilities as a tag
    // list, primarily for display/hints.
    debug: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let component_path = args.next().ok_or_else(|| {
        anyhow!("usage: actions-catalog-gen <path-to-action-executor-component.wasm> [output.json]")
    })?;

    let output_path = args.next().map(PathBuf::from);

    let component_path = PathBuf::from(component_path);
    let (schema_version, actions) = load_catalog_from_component(&component_path).await?;

    let catalog = Catalog {
        schema_version,
        executor: ExecutorInfo {
            component_path: component_path.display().to_string(),
        },
        actions,
    };

    let json = serde_json::to_string_pretty(&catalog).context("serialize catalog")?;

    match output_path {
        Some(p) => {
            std::fs::write(&p, json).with_context(|| format!("write {}", p.display()))?;
        }
        None => {
            println!("{json}");
        }
    }

    Ok(())
}

async fn load_catalog_from_component(component_path: &Path) -> Result<(u32, Vec<ActionEntry>)> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model_async(true);
    config.async_support(true);
    let engine = Engine::new(&config)?;

    let component = Component::from_file(&engine, component_path)
        .with_context(|| format!("load component {}", component_path.display()))?;

    let mut linker = Linker::<HostState>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;

    // NOTE: The actions-executor world imports `event-bus`. For this tool we only
    // call the *metadata* APIs, but we still must satisfy the import at instantiation.
    // If the import shape changes, this tool will need to be updated accordingly.
    crate::ntx::scenario_eventbus::event_bus::add_to_linker::<_, HasSelf<HostState>>(
        &mut linker,
        |s: &mut HostState| s,
    )
    .context("wire event-bus import")?;

    // Minimal WASI ctx.
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

    // Call catalog APIs.
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

        let summary_json = ActionSummaryJson {
            id: summary.id.clone(),
            title: summary.title.clone(),
            description: summary.description.clone().unwrap_or_default(),
        };

        let spec_json = ActionSpecJson {
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
        };

        actions.push(ActionEntry {
            summary: summary_json,
            spec: spec_json,
        });
    }

    // Keep output stable (id sort).
    actions.sort_by(|a, b| a.summary.id.cmp(&b.summary.id));

    Ok((schema_version, actions))
}
