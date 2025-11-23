wit_bindgen::generate!({
    path: ["../wit/core-libs"],
    world: "core-libs",
    generate_all,
    debug: true,
});

mod call_model;
mod logger;
mod network;
mod progress;
mod random;
mod timer;

use call_model::CallModel;
pub struct CoreLib;

export!(CoreLib);
