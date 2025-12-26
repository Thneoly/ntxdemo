wit_bindgen::generate!({
    world: "ntx:core-types/core-types@0.1.0",
    path: ["../wit/types"],
    generate_all,
    generate_unused_types:true,
    debug: true,
});
