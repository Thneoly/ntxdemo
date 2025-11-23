wit_bindgen::generate!({
    path: ["../wit/core-libs"],
    world: "core-libs",
    generate_all,
    debug: true,
});

struct CoreLib;

export!(CoreLib);

impl Guest for CoreLib {}
