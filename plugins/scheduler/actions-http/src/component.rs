// Component bindings for scheduler-actions-http
#[cfg(target_arch = "wasm32")]
mod bindings {
    use crate::{HttpActionComponent, extract_body, extract_headers, extract_url};
    use indexmap::IndexMap;
    use scheduler_core::dsl::ActionDef;
    use scheduler_executor::{ActionComponent, ActionOutcome};

    wit_bindgen::generate!({
        world: "http-action-component",
        path: "../wit",
    });

    struct HttpActionComponentImpl {
        component: HttpActionComponent,
    }

    impl HttpActionComponentImpl {
        fn new() -> Self {
            Self {
                component: HttpActionComponent::default(),
            }
        }
    }

    impl Guest for HttpActionComponentImpl {
        fn init_component() -> Result<(), String> {
            Ok(())
        }

        fn do_http_action(
            action: exports::types::ActionDef,
        ) -> Result<exports::types::ActionOutcome, String> {
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
                return Ok(exports::types::ActionOutcome {
                    status: exports::types::ActionStatus::Success,
                    detail: Some(format!("skip unresolved template url={}", url)),
                });
            }

            // For wasm32, we'd use wasi:http imports instead of ureq
            // This is a simplified stub
            Ok(exports::types::ActionOutcome {
                status: exports::types::ActionStatus::Success,
                detail: Some(format!("HTTP {} {} (wasm stub)", method, url)),
            })
        }

        fn release_component() -> Result<(), String> {
            Ok(())
        }
    }

    export!(HttpActionComponentImpl);
}
