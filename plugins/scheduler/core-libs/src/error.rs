use anyhow::Error as AnyError;
use ctrlc::Error as CtrlcError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("failed to parse scenario: {0}")]
    Parse(#[from] serde_yaml::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("unknown action `{action}` referenced by workflow node `{node}`")]
    UnknownAction { action: String, node: String },

    #[error("unknown workflow node `{0}` referenced by edge")]
    UnknownNode(String),

    #[error("task `{0}` not found")]
    TaskNotFound(String),

    #[error("action `{0}` not registered")]
    ActionNotRegistered(String),

    #[error("action `{action}` failed: {source}")]
    ActionExecution {
        action: String,
        #[source]
        source: AnyError,
    },

    #[error("failed to initialize action component: {source}")]
    ActionComponentInit {
        #[source]
        source: AnyError,
    },

    #[error("failed to release action component: {source}")]
    ActionComponentRelease {
        #[source]
        source: AnyError,
    },

    #[error("failed to register signal handler: {source}")]
    SignalHandler {
        #[source]
        source: CtrlcError,
    },
}
