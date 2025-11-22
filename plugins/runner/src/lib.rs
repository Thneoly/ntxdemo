mod macros;
mod mywasi;
mod polling;
mod runtime;
mod time;
use crate::runtime::block_on;

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
        block_on(|_| async move {
            println!("Running select_index example...");

            // Example 1: select between three monotonic-clock subscriptions (Pollable)
            // The first to fire will be the smallest duration (1 second)
            let idx = select_index!(
                crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration(1_000_000_000), // 1 second
                crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration(2_000_000_000), // 2 seconds
                crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration(3_000_000_000), // 3 seconds
            )
            .await;

            match idx {
                0 => println!("First pollable (1s) completed first"),
                1 => println!("Second pollable (2s) completed first"),
                2 => println!("Third pollable (3s) completed first"),
                _ => unreachable!(),
            }

            // Example 2: demonstrate multiple selects
            for i in 0..3 {
                println!("Select iteration: {}", i + 1);
                let idx = select_index!(
                    crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration(500_000_000), // 0.5 seconds
                    crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration(1_000_000_000), // 1 second
                )
                .await;

                match idx {
                    0 => println!("  Faster timer (0.5s) completed first"),
                    1 => println!("  Slower timer (1s) completed first"),
                    _ => unreachable!(),
                }
            }

            println!("Select examples completed!");
        });
    }
}

export!(Component);
