use chrono::Local;
use std::io;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::{
    self, fmt::time::FormatTime, layer::SubscriberExt, util::SubscriberInitExt,
};

#[derive(Clone)]
struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", Local::now().format("%FT%T%.3f"))
    }
}

pub fn logger_init() {
    let format = tracing_subscriber::fmt::format()
        .with_level(true)
        .with_target(true)
        .with_timer(LocalTimer)
        .with_file(true)
        .with_line_number(true);

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stdout)
        .with_ansi(false)
        .event_format(format.clone());

    // In some test/CI environments, even `/tmp` can be permission restricted.
    // If the rolling file appender can't be initialized, fall back to stdout-only.
    let writer_layer = match tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("tracing")
        .filename_suffix("log")
        .build("/tmp")
    {
        Ok(file_appender) => {
            let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
            Some(
                tracing_subscriber::fmt::layer()
                    .with_writer(non_blocking)
                    .event_format(format),
            )
        }
        Err(_) => None,
    };

    let registry = tracing_subscriber::registry()
        .with(stdout_layer)
        .with(tracing_subscriber::filter::LevelFilter::TRACE);

    if let Some(file_layer) = writer_layer {
        registry.with(file_layer).init();
    } else {
        registry.init();
    }
}
