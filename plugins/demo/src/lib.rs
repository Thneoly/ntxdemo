use wit_bindgen::generate;
generate!({
    world: "core",
    path: ["../wit",],
    generate_all,
});

struct Demo;

export!(Demo);
