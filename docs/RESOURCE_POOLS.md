# Resource Pools (IP/MAC/Port)

This repo includes a small, config-driven resource pool module in `ntx-network`:

- `ntx::network::resources::ResourcePoolsConfig`
- `ntx::network::resources::ResourcePools`

It’s meant for demos/examples and higher-level components that need a simple **startup-time resource budget** (e.g. IP pool, MAC pool, ephemeral port pool).

## Config format (YAML)

Example `resources.yaml`:

```yaml
ipv4:
  - name: demo
    cidr: "10.0.0.0/24"
    exclude:
      - "10.0.0.1"

mac:
  - name: demo
    start: "02:00:00:00:00:00"
    end:   "02:00:00:00:00:ff"

udp_port:
  - name: ephemeral
    start: 40000
    end: 40100
    exclude: [40001]

tcp_port:
  - name: service
    start: 8080
    end: 8080
```

## Usage

```rust
use anyhow::Result;
use ntx::network::resources::ResourcePoolsConfig;

fn main() -> Result<()> {
    let cfg = ResourcePoolsConfig::load_yaml_file("resources.yaml")?;
    let mut pools = cfg.build()?;

  // Named pools: access by name.
  // If a config entry omits `name`, it is grouped under "default".
  let ip = pools
    .ipv4("demo")
    .expect("missing ipv4 pool")
    .acquire()
    .expect("no IPs left");
  let mac = pools
    .mac("demo")
    .expect("missing mac pool")
    .acquire()
    .expect("no MACs left");
  let port = pools
    .udp_port("ephemeral")
    .expect("missing udp port pool")
    .acquire()
    .expect("no ports left");

    // ... use resources ...

  pools.udp_port.release(port);
    pools.mac.release(mac);
    pools.ipv4.release(ip);
    Ok(())
}
```

## Notes / semantics

- IPv4 CIDR expansion:
  - `/32` yields a single address
  - `/31` yields two usable addresses
  - `/30` or larger networks exclude network/broadcast by default (host-range semantics)
- Allocation is deterministic (sorted).

### TCP vs UDP port pools

Use top-level keys:

- `udp_port:` for UDP port pools
- `tcp_port:` for TCP port pools

For backward compatibility, `port:` is still accepted and is treated as **UDP**.

## Pin / reserve (sticky assignments)

Each pool supports pinning a specific resource to an owner id (e.g. component id):

```rust
let ip_pool = pools.ipv4("demo").unwrap();
ip_pool.pin("comp-a", ip)?;
let ip_again = ip_pool.acquire_for("comp-a").unwrap();
assert_eq!(ip_again, ip);
```

While pinned, `release()` will not return that resource to the general pool.
Call `unpin_owner("comp-a")` to remove the pin.

### Multiple pinned ports per owner

`PortPool::pin(owner, port)` can be called multiple times for the same `owner` to pin multiple ports.
`publish_abr_for_owner(...)` will publish **all pinned ports** (TCP and UDP) for that owner into ABR.
