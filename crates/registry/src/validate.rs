use anyhow::{anyhow, Result};

#[cfg(feature = "validate")]
use jsonschema::is_valid as jsonschema_is_valid;
// component validation with wasmtime is deferred

use crate::RegistryError;

pub async fn validate_manifest_schema(
    json: &serde_json::Value,
    schema: &serde_json::Value,
) -> Result<()> {
    #[cfg(feature = "validate")]
    {
        if !jsonschema_is_valid(schema, json) {
            return Err(anyhow!(RegistryError::Validate(
                "manifest schema validation failed".into(),
            )));
        }
        Ok(())
    }
    #[cfg(not(feature = "validate"))]
    {
        let _ = json;
        let _ = schema;
        Err(anyhow!(RegistryError::Validate(
            "validate feature not enabled".into()
        )))
    }
}

pub async fn validate_component_bytes(bytes: &[u8]) -> Result<()> {
    #[cfg(feature = "validate")]
    {
        let _ = bytes;
        // TODO: enable wasmtime component validation behind a separate feature
        Ok(())
    }
    #[cfg(not(feature = "validate"))]
    {
        let _ = bytes;
        Err(anyhow!(RegistryError::Validate(
            "validate feature not enabled".into()
        )))
    }
}
