/// Example: Using IP Pool with HTTP Actions
///
/// This example demonstrates how to integrate IP pool management with HTTP requests,
/// allowing actions to bind to specific source IPs.
use scheduler_core::{IpPool, ResourceType};
use std::net::IpAddr;

fn main() -> anyhow::Result<()> {
    println!("=== IP Pool Integration with HTTP Actions ===\n");

    // 1. Create and configure IP pool
    let mut pool = IpPool::new("http-client-pool");
    pool.add_cidr_range("10.0.1.0/24")?;
    pool.add_cidr_range("10.0.2.0/24")?;

    println!("✓ Created IP pool with ranges:");
    println!("  - 10.0.1.0/24 (254 IPs)");
    println!("  - 10.0.2.0/24 (254 IPs)");
    println!();

    // 2. Allocate IPs for different services/tenants
    let tenant_a_ip = pool.allocate(
        "service-a",
        "http-client-1",
        ResourceType::Custom("http-client".into()),
    )?;
    println!("✓ Allocated IP for Tenant A: {}", tenant_a_ip);

    let tenant_b_ip = pool.allocate(
        "service-b",
        "http-client-2",
        ResourceType::Custom("http-client".into()),
    )?;
    println!("✓ Allocated IP for Tenant B: {}", tenant_b_ip);
    println!();

    // 3. Show how to use IPs in HTTP actions

    // Example 1: DSL format with bind_ip
    println!("Example 1: DSL Action with IP binding");
    println!("---------------------------------------");
    print_dsl_example(&tenant_a_ip);
    println!();

    // Example 2: JSON format for component
    println!("Example 2: Component Action with IP binding");
    println!("--------------------------------------------");
    print_json_example(&tenant_b_ip);
    println!();

    // Example 3: Multiple IPs for different actions
    println!("Example 3: Multi-tenant scenario");
    println!("---------------------------------");

    // Allocate more IPs
    let api1_ip = pool.allocate(
        "api-gateway",
        "worker-1",
        ResourceType::Pod("api-gw-1".into()),
    )?;
    let api2_ip = pool.allocate(
        "api-gateway",
        "worker-2",
        ResourceType::Pod("api-gw-2".into()),
    )?;

    println!("API Gateway IPs:");
    println!("  Worker 1: {}", api1_ip);
    println!("  Worker 2: {}", api2_ip);
    println!();

    print_multi_tenant_example(&api1_ip, &api2_ip);
    println!();

    // 4. Show binding information
    println!("IP Pool Status:");
    println!("---------------");
    let stats = pool.stats();
    println!("Total allocated: {}", stats.allocated);
    println!("Total available: {}", stats.available);
    println!();

    if let Some(binding) = pool.get_binding(&tenant_a_ip) {
        println!("Binding details for {}:", tenant_a_ip);
        println!("  Subinstance: {}", binding.subinstance);
        println!("  Subid: {}", binding.subid);
        println!("  Type: {}", binding.subtype.type_name());
        println!("  Resource: {}", binding.subtype.as_str());
    }
    println!();

    // 5. Release IPs when done
    pool.release_by_ip(&tenant_a_ip)?;
    println!("✓ Released IP: {}", tenant_a_ip);
    let stats = pool.stats();
    println!("  Available IPs: {}", stats.available);

    Ok(())
}

fn print_dsl_example(ip: &IpAddr) {
    println!(
        r#"
actions:
  - id: fetch-with-source-ip
    call: GET
    with:
      url: "http://api.example.com/data"
      bind_ip: "{}"
      headers:
        User-Agent: "Scheduler-Client"
        Accept: "application/json"
    export:
      - type: variable
        name: api_response
        scope: step
"#,
        ip
    );
}

fn print_json_example(ip: &IpAddr) {
    println!(
        r#"
{{
  "id": "http-post-action",
  "call": "POST",
  "with-params": "{{\"url\":\"http://10.0.0.1:8080/submit\",\"bind_ip\":\"{}\",\"headers\":{{\"Content-Type\":\"application/json\"}},\"body\":\"{{\\\"data\\\":\\\"test\\\"}}\"}}",
  "exports": []
}}
"#,
        ip
    );
}

fn print_multi_tenant_example(ip1: &IpAddr, ip2: &IpAddr) {
    println!(
        r#"
# Tenant A requests using IP {}
actions:
  - id: tenant-a-api-call
    call: GET
    with:
      url: "http://backend.internal:8080/tenant-a/data"
      bind_ip: "{}"

# Tenant B requests using IP {}  
  - id: tenant-b-api-call
    call: GET
    with:
      url: "http://backend.internal:8080/tenant-b/data"
      bind_ip: "{}"
"#,
        ip1, ip1, ip2, ip2
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ip_pool_allocation() {
        let mut pool = IpPool::new("test");
        pool.add_cidr_range("192.168.1.0/24").unwrap();

        let ip1 = pool
            .allocate("svc", "client1", ResourceType::Custom("http".into()))
            .unwrap();
        let ip2 = pool
            .allocate("svc", "client2", ResourceType::Custom("http".into()))
            .unwrap();

        assert_ne!(ip1, ip2);
        let stats = pool.stats();
        assert_eq!(stats.allocated, 2);
    }

    #[test]
    fn test_ip_binding_retrieval() {
        let mut pool = IpPool::new("test");
        pool.add_cidr_range("10.0.0.0/24").unwrap();

        let ip = pool
            .allocate("service-a", "worker-1", ResourceType::Pod("pod-1".into()))
            .unwrap();

        let binding = pool.get_binding(&ip).unwrap();
        assert_eq!(binding.subinstance, "service-a");
        assert_eq!(binding.subid, "worker-1");
        assert_eq!(binding.subtype, ResourceType::Pod("pod-1".into()));
    }
}
