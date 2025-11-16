```shell
cargo binstall wac-cli

cargo component add wasi:http@0.2.8
cargo component update
cargo component build --target wasm32-wasip2

cargo expand
wit-bindgen rust --generate-all wit/
cargo build --target wasm32-wasip2

wasmtime run --invoke "fun()" target/wasm32-wasip2/debug/http_send.wasm 
>> "Hello, world!"


wasmtime run --wasi help

nc -l 8080

./build.sh
./run.sh
wasmtime run -S tcp=y -S inherit-network=y --invoke="start()" ./http_send.wasm

```

```shell
nc（Netcat）是一个功能强大的网络工具，常被称为“网络瑞士军刀”，它可以用于创建 TCP 或 UDP 连接、端口扫描、文件传输、聊天、反弹 shell 等操作。它支持多种平台，如 Linux、Unix 和 Windows（Windows 版本常称为 ncat）。nc 的设计简单，但用途广泛，常用于网络调试、安全测试和数据传输。
基本语法
nc 的基本命令格式为：
textnc [选项] [主机] [端口]
常见选项包括：

-l：监听模式（作为服务器）。
-p <port>：指定端口（在监听模式下使用）。
-v：详细输出（显示更多信息）。
-u：使用 UDP 协议（默认是 TCP）。
-z：端口扫描模式（不发送数据，只检查端口是否开放）。
-e <program>：执行程序（用于反弹 shell，但某些版本不支持）。
-k：保持监听（接受多个连接）。
-n：不进行 DNS 解析（使用 IP 地址）。
-w <seconds>：超时时间。

常见用法示例

作为服务器监听端口（echo server 示例）：textnc -l -p 8080这会在本地 8080 端口监听 TCP 连接。客户端连接后，可以输入数据，服务器会回显。
作为客户端连接服务器：textnc 127.0.0.1 8080连接到本地 8080 端口。可以输入数据发送给服务器。
端口扫描：textnc -v -z 192.168.1.1 1-1000扫描指定主机 1 到 1000 端口，显示开放端口。
文件传输：
服务器端（接收文件）：textnc -l -p 8080 > output.file
客户端端（发送文件）：textnc 127.0.0.1 8080 < input.file
这可以将文件从客户端传输到服务器。
简单聊天工具：
一方监听：nc -l -p 8080
另一方连接：nc 127.0.0.1 8080
双方可以输入消息实时聊天。

反弹 shell（安全测试中使用，需谨慎）：
目标机（监听）：nc -l -p 8080 -e /bin/bash（某些版本不支持 -e，可用管道替代）。
攻击机：nc 目标IP 8080
这可以获取远程 shell。

UDP 模式：textnc -u -l -p 8080  # 服务器 UDP 监听
nc -u 127.0.0.1 8080  # 客户端连接用于 UDP 数据传输。
```