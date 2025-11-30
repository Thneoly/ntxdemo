/// Socket with IP Pool Integration Examples
///
/// This example demonstrates how to use the Socket API with IP pool management,
/// showing the complete flow: allocate IP -> bind socket -> send/recv
use scheduler_core::{IpPool, ResourceType, Socket, SocketAddress};
use std::net::IpAddr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Socket with IP Pool Integration Examples ===\n");

    // Example 1: TCP Server with IP Pool
    example_1_tcp_server()?;

    // Example 2: TCP Client with specific IP
    example_2_tcp_client()?;

    // Example 3: UDP Socket with IP binding
    example_3_udp_socket()?;

    // Example 4: Multi-tenant socket management
    example_4_multi_tenant()?;

    Ok(())
}

fn example_1_tcp_server() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Example 1: TCP Server with IP Pool ---");

    // Step 1: Create IP pool and allocate an IP
    let mut pool = IpPool::new("server-pool");
    pool.add_cidr_range("192.168.1.0/24")?;

    let server_ip = pool.allocate(
        "web-services",
        "http-server-1",
        ResourceType::Custom("tcp-server".to_string()),
    )?;

    println!("Allocated IP for server: {}", server_ip);

    // Step 2: Create TCP socket
    let mut server_socket = Socket::tcp_v4()?;
    println!("Created TCP socket: handle {}", server_socket.handle());

    // Step 3: Bind socket to the allocated IP
    server_socket.bind_to_ip(server_ip, 8080)?;
    println!(
        "Bound socket to {}:{}",
        server_socket.local_ip().unwrap(),
        server_socket.local_port().unwrap()
    );

    // Step 4: Listen for connections
    server_socket.listen(128)?;
    println!("Listening for connections (backlog: 128)");

    // In a real scenario, you would accept connections:
    // let client = server_socket.accept()?;

    // Cleanup
    server_socket.close()?;
    pool.release_by_subid("web-services", "http-server-1")?;
    println!("Server closed and IP released\n");

    Ok(())
}

fn example_2_tcp_client() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Example 2: TCP Client with Specific Source IP ---");

    // Allocate IP for client
    let mut pool = IpPool::new("client-pool");
    pool.add_cidr_range("10.0.0.0/16")?;

    let client_ip = pool.allocate(
        "app-clients",
        "client-001",
        ResourceType::Container("app-container-1".to_string()),
    )?;

    println!("Allocated client IP: {}", client_ip);

    // Create and bind client socket
    let mut client_socket = Socket::tcp_v4()?;

    // Bind to specific source IP (optional for clients, but useful for routing)
    client_socket.bind_to_ip(client_ip, 0)?; // Port 0 = let OS choose
    println!(
        "Client bound to source IP: {}",
        client_socket.local_ip().unwrap()
    );

    // Connect to server
    let server_addr = SocketAddress::new("192.168.1.1", 8080);
    println!("Connecting to {}:{}", server_addr.host, server_addr.port);

    // In WASM environment, this would establish connection
    // client_socket.connect(server_addr)?;

    // Send data (after connection is established)
    // let data = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
    // let sent = client_socket.send(data)?;
    // println!("Sent {} bytes", sent);

    // Cleanup
    client_socket.close()?;
    pool.release_by_subid("app-clients", "client-001")?;
    println!("Client closed\n");

    Ok(())
}

fn example_3_udp_socket() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Example 3: UDP Socket with IP Binding ---");

    // Allocate IP for UDP service
    let mut pool = IpPool::new("udp-pool");
    pool.add_cidr_range("172.16.0.0/16")?;

    let udp_ip = pool.allocate(
        "monitoring",
        "metrics-collector",
        ResourceType::Pod("metrics-pod-abc".to_string()),
    )?;

    println!("Allocated UDP IP: {}", udp_ip);

    // Create UDP socket
    let mut udp_socket = Socket::udp_v4()?;
    println!("Created UDP socket");

    // Bind to IP and port
    udp_socket.bind_to_ip(udp_ip, 9125)?; // StatsD port
    println!("UDP socket bound to {}:9125", udp_ip);

    // UDP can send/receive without connect
    // Receive data:
    // let data = udp_socket.recv(1024)?;

    // Send to specific address:
    // let target = SocketAddress::new("172.16.0.10", 8125);
    // udp_socket.send_to(b"metric:1|c", target)?;

    // Receive from any address:
    // let (data, sender) = udp_socket.recv_from(1024)?;

    udp_socket.close()?;
    pool.release_by_subid("monitoring", "metrics-collector")?;
    println!("UDP socket closed\n");

    Ok(())
}

fn example_4_multi_tenant() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Example 4: Multi-Tenant Socket Management ---");

    let mut pool = IpPool::new("multi-tenant");
    pool.add_cidr_range("10.100.0.0/16")?;

    // Tenant A - Web service
    let tenant_a_ip = pool.allocate(
        "tenant-a",
        "web-service",
        ResourceType::Vm("web-vm-001".to_string()),
    )?;

    let mut tenant_a_socket = Socket::tcp_v4()?;
    tenant_a_socket.bind_to_ip(tenant_a_ip, 80)?;
    tenant_a_socket.listen(100)?;

    println!(
        "Tenant A: web service on {}:80",
        tenant_a_socket.local_ip().unwrap()
    );

    // Tenant A - Database
    let tenant_a_db_ip = pool.allocate(
        "tenant-a",
        "database",
        ResourceType::Vm("db-vm-001".to_string()),
    )?;

    let mut tenant_a_db_socket = Socket::tcp_v4()?;
    tenant_a_db_socket.bind_to_ip(tenant_a_db_ip, 5432)?;
    tenant_a_db_socket.listen(50)?;

    println!(
        "Tenant A: database on {}:5432",
        tenant_a_db_socket.local_ip().unwrap()
    );

    // Tenant B - API service
    let tenant_b_ip = pool.allocate(
        "tenant-b",
        "api-service",
        ResourceType::Container("api-container".to_string()),
    )?;

    let mut tenant_b_socket = Socket::tcp_v4()?;
    tenant_b_socket.bind_to_ip(tenant_b_ip, 8080)?;
    tenant_b_socket.listen(200)?;

    println!(
        "Tenant B: API service on {}:8080",
        tenant_b_socket.local_ip().unwrap()
    );

    // Show pool stats
    let stats = pool.stats();
    println!(
        "\nPool stats: {}/{} IPs allocated",
        stats.allocated, stats.total
    );

    // List IPs by tenant
    let tenant_a_ips = pool.list_by_subinstance("tenant-a");
    println!("Tenant A has {} IPs", tenant_a_ips.len());

    let tenant_b_ips = pool.list_by_subinstance("tenant-b");
    println!("Tenant B has {} IPs", tenant_b_ips.len());

    // Cleanup tenant A
    tenant_a_socket.close()?;
    tenant_a_db_socket.close()?;
    let released = pool.release_by_subinstance("tenant-a");
    println!("\nReleased {} IPs from tenant-a", released.len());

    // Cleanup tenant B
    tenant_b_socket.close()?;
    pool.release_by_subinstance("tenant-b");

    println!();
    Ok(())
}

/// Advanced example: HTTP-like request/response pattern
#[allow(dead_code)]
fn example_http_pattern() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Advanced: HTTP-like Request/Response Pattern ---");

    // This demonstrates the foundation for building an HTTP component

    // 1. Allocate IP for HTTP server
    let mut pool = IpPool::new("http-pool");
    pool.add_cidr_range("192.168.100.0/24")?;

    let http_ip = pool.allocate(
        "http-services",
        "http-server-main",
        ResourceType::Custom("http-server".to_string()),
    )?;

    // 2. Create server socket
    let mut server = Socket::tcp_v4()?;
    server.bind_to_ip(http_ip, 80)?;
    server.listen(1000)?;

    println!("HTTP server listening on {}:80", http_ip);

    // 3. Accept client connections (in a real server, this would be in a loop)
    // let mut client = server.accept()?;

    // 4. Receive HTTP request
    // let request = client.recv(4096)?;
    // let request_str = String::from_utf8_lossy(&request);

    // 5. Parse HTTP request (future HTTP component will handle this)
    // let (method, path, headers, body) = parse_http_request(&request_str);

    // 6. Process request and generate response
    // let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nHello World";

    // 7. Send response
    // client.send(response)?;

    // 8. Close connection
    // client.close()?;

    server.close()?;
    pool.release_by_subid("http-services", "http-server-main")?;

    println!("HTTP server pattern demonstrated\n");

    Ok(())
}

/// Example showing socket reuse with IP pool
#[allow(dead_code)]
fn example_socket_reuse() -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Example: Socket IP Reuse ---");

    let mut pool = IpPool::new("reuse-pool");
    pool.add_cidr_range("10.200.0.0/24")?;

    // Allocate IP
    let ip = pool.allocate(
        "services",
        "api-v1",
        ResourceType::Custom("api".to_string()),
    )?;

    println!("Allocated IP: {}", ip);

    // First socket
    {
        let mut sock1 = Socket::tcp_v4()?;
        sock1.bind_to_ip(ip, 8000)?;
        println!("Socket 1 bound to {}:8000", ip);
        sock1.close()?;
        println!("Socket 1 closed");
    } // Socket 1 is dropped here

    // After closing, we can reuse the IP with a new socket
    {
        let mut sock2 = Socket::tcp_v4()?;
        sock2.bind_to_ip(ip, 8000)?;
        println!("Socket 2 bound to {}:8000 (reused)", ip);
        sock2.close()?;
    }

    // IP remains allocated in pool until explicitly released
    pool.release_by_subid("services", "api-v1")?;
    println!("IP released from pool\n");

    Ok(())
}
