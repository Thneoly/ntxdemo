use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub error: String,
}
