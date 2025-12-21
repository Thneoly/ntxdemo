use anyhow::Result;
use ntx::{kernel, logger, scheduler};

fn main() -> Result<()> {
    logger::logger_init();
    kernel::init("config/config.yaml")?;

    scheduler::init();
    scheduler::start_scheduler()?;
    Ok(())
}
