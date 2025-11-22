mod mywasi;
mod polling;
mod runtime;
mod time;
use crate::runtime::block_on;
use crate::time::sleep;
use std::time::Duration;

wit_bindgen::generate!({
    path: "wit",
    world: "runner",
    async: false,
    debug: true,
    generate_all,
});
struct Component;

impl Guest for Component {
    fn start() {
        block_on(|reactor| async move {
            loop {
                sleep(Duration::from_secs(1), &reactor).await;
                println!("Runner is alive!");
            }
        });
    }
}

export!(Component);
