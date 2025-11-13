wit_bindgen::generate!({
    path: ["wit"],
    world: "example",
    generate_all,
});

struct Protocol;

impl Guest for Protocol {
    fn fun() -> String {
        "Hello, world!".to_string()
    }
}

export!(Protocol);