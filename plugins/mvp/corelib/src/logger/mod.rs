use crate::CoreLib;
use crate::exports::ntx::runner::core_logger::Guest;
use crate::exports::ntx::runner::core_logger::LogLevel;
use crate::ntx::runner::types::{ActionId, TaskId, UserId};
impl Guest for CoreLib {
    fn log(
        level: LogLevel,
        message: String,
        task: Option<TaskId>,
        user: Option<UserId>,
        action: Option<ActionId>,
    ) {
        match level {
            LogLevel::Trace => println!("{}", message),
            LogLevel::Debug => println!("{}", message),
            LogLevel::Info => println!("{}", message),
            LogLevel::Warn => println!("{}", message),
            LogLevel::Error => println!("{}", message),
            LogLevel::Critical => println!("{}", message),
        }
    }
}
