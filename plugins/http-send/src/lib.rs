wit_bindgen::generate!({
    path: ["wit"],
    world: "example",
    generate_all,
});

use wasi::sockets::instance_network::instance_network;
use wasi::sockets::network::{ErrorCode, Ipv4SocketAddress, Network};
use wasi::sockets::tcp::IpSocketAddress;
use wasi::sockets::tcp_create_socket::{IpAddressFamily, TcpSocket, create_tcp_socket};

struct Protocol;

impl Guest for Protocol {
    fn start() -> String {
        let sock: TcpSocket = match create_tcp_socket(IpAddressFamily::Ipv4) {
            Ok(s) => s,
            Err(e) => panic!("Failed to create socket: {}", e),
        };

        // 注意：如果你只是作为客户端连接，通常不需要显式绑定本地端口，
        // 操作系统会自动分配一个。除非你需要绑定到特定的本地地址/端口。
        // 如果确实需要绑定，也需要处理 finish_bind 的 would-block。
        /*
        let bind_address = IpSocketAddress::Ipv4(Ipv4SocketAddress {
            address: (127, 0, 0, 1),
            port: 8081, // 或 0 让系统分配
        });
        let network: Network = instance_network();
        sock.start_bind(&network, bind_address).expect("Failed to start bind");
        // 需要循环处理 finish_bind 的 would-block
        loop {
             match sock.finish_bind() {
                 Ok(()) => break,
                 Err(ErrorCode::WouldBlock) => { /* 可以短暂休眠或直接重试 */ },
                 Err(e) => panic!("Failed to finish bind: {:?}", e),
             }
        }
        */

        let remote = IpSocketAddress::Ipv4(Ipv4SocketAddress {
            address: (127, 0, 0, 1),
            port: 8080,
        });

        let network: Network = instance_network();

        // Start the connection attempt
        match sock.start_connect(&network, remote) {
            Ok(()) => println!("Started connecting..."),
            Err(error) => {
                println!("Error starting connect: {}", error);
                panic!("Failed to start connect {}", error)
            }
        }

        // Poll until the connection is established or fails permanently
        let (input, output) = loop {
            match sock.finish_connect() {
                Ok(streams) => {
                    println!("Connected successfully!");
                    break streams; // Exit the loop with the streams
                }
                Err(ErrorCode::WouldBlock) => {
                    // Connection is still in progress.
                    // In a real application, you might wait or yield here.
                    // For simplicity, we just keep polling rapidly.
                    // Consider adding a small delay or using async mechanisms if available.
                    // std::thread::sleep(std::time::Duration::from_millis(1)); // Optional short delay
                    // println!("Would block, retrying...");
                }
                Err(error) => {
                    println!("Error finishing connect: {}", error);
                    panic!("Failed to finish connect: {}", error);
                }
            }
        };

        // Now you can use input and output
        match output.write(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n") {
            Ok(_) => println!("Request sent."),
            Err(e) => println!("Failed to send request: {:?}", e),
        }

        match input.read(1024) {
            Ok(rcv_bytes) => {
                println!("Received {:#?} bytes", rcv_bytes);
                // Process the response in buffer[..rcv_bytes]
                let response_str = String::from_utf8_lossy(&rcv_bytes);
                println!("Response:\n{}", response_str);
            }
            Err(e) => println!("Failed to read response: {:?}", e),
        }

        "Hello, Connected World!".to_string() // Or return relevant data
    }
}

export!(Protocol);
