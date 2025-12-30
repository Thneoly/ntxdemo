use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub world: Option<String>,
    #[serde(default)]
    pub encodings: Option<Vec<String>>, // simplified
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub name: String,
    pub version: String,
    pub component: ReleaseComponent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseComponent {
    #[serde(rename = "sha256", default)]
    pub digest_sha256: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub url: Option<String>,
}
