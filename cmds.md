```shell
cargo component add wasi:http@0.2.8
cargo component update
cargo component build --target wasm32-wasip2

cargo expand
wit-bindgen rust --generate-all wit/
cargo build --target wasm32-wasip2

wasmtime run --invoke "fun()" target/wasm32-wasip2/debug/http_send.wasm 
>> "Hello, world!"
```