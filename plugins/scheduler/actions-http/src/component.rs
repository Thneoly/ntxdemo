// Component bindings for scheduler-actions-http
#[cfg(target_arch = "wasm32")]
mod bindings {
    use crate::extract_url;
    use indexmap::IndexMap;
    use scheduler_core::dsl::ActionDef;

    wit_bindgen::generate!({
        world: "http-action-component",
        path: "wit",
    });

    struct HttpActionComponentImpl;

    impl exports::scheduler::actions_http::http_component::Guest for HttpActionComponentImpl {
        fn init_component() -> Result<(), String> {
            Ok(())
        }

        fn do_http_action(
            action: exports::scheduler::actions_http::types::ActionDef,
        ) -> Result<exports::scheduler::actions_http::types::ActionOutcome, String> {
            // Convert wit action to native ActionDef
            let with_params: IndexMap<String, serde_yaml::Value> =
                serde_json::from_str(&action.with_params)
                    .map_err(|e| format!("failed to parse with-params: {}", e))?;

            let native_action = ActionDef {
                id: action.id.clone(),
                call: action.call.clone(),
                with: with_params,
                export: action
                    .exports
                    .into_iter()
                    .map(|e| scheduler_core::dsl::ExportDef {
                        export_type: e.export_type,
                        name: e.name,
                        scope: e.scope,
                        default: e.default_value.and_then(|v| serde_json::from_str(&v).ok()),
                    })
                    .collect(),
            };

            // Execute HTTP request
            let method = native_action.call.to_uppercase();
            let url =
                extract_url(&native_action).map_err(|e| format!("failed to extract url: {}", e))?;

            if url.contains("{{") {
                return Ok(exports::scheduler::actions_http::types::ActionOutcome {
                    status: exports::scheduler::actions_http::types::ActionStatus::Success,
                    detail: Some(format!("skip unresolved template url={}", url)),
                });
            }

            // For wasm32, we'd use wasi:http imports instead of ureq
            // This is a simplified stub
            Ok(exports::scheduler::actions_http::types::ActionOutcome {
                status: exports::scheduler::actions_http::types::ActionStatus::Success,
                detail: Some(format!("HTTP {} {} (wasm stub)", method, url)),
            })
        }

        fn release_component() -> Result<(), String> {
            Ok(())
        }
    }

    export!(HttpActionComponentImpl);
}
