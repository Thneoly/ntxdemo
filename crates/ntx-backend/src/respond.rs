use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::types::ErrorBody;

pub fn json_error(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(ErrorBody { error: msg.into() })).into_response()
}
