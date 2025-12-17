# ntx-echo (userspace)

This folder contains a minimal **userspace** MAC/ARP + IPv4/UDP echo client/server built on top of the `ntx-network` crate and Linux `AF_PACKET`.

Topology is provided by `scripts/ntx-veth-up.sh`:

- Host namespace: `ntx0` = `10.0.0.1/24`
- Netns `ntxns1`: `ntx1` = `10.0.0.2/24`

The echo server listens on the traditional UDP echo port **7**.

## Bring up veth + netns

```bash
sudo ./scripts/ntx-veth-up.sh
```

## Run server (in netns, iface=ntx1)

```bash
sudo ./examples/ntx-echo/scripts/run-server.sh
```

Equivalent manual commands:

```bash
cargo build --example ntx-echo-server
sudo ip netns exec ntxns1 ./target/debug/examples/ntx-echo-server ntx1
```

Alternative (run via cargo directly; may fail in netns if PATH doesn’t include cargo):

```bash
sudo ip netns exec ntxns1 cargo run --example ntx-echo-server -- ntx1
```

## Run client (host, iface=ntx0)

```bash
sudo ./examples/ntx-echo/scripts/run-client.sh
```

Equivalent manual commands:

```bash
cargo build --example ntx-echo-client
sudo ./target/debug/examples/ntx-echo-client ntx0
```

Alternative (run via cargo directly):

```bash
sudo cargo run --example ntx-echo-client -- ntx0
```

Expected output:

- client prints an ARP resolution line and the received UDP payload
- server stays running and responds to ARP and UDP echo requests

## Notes

- Running requires `root` (or `cap_net_raw`) because it uses `AF_PACKET` raw sockets.
- These examples do not rely on the kernel IP stack; they build/parse Ethernet/ARP/IPv4/UDP in userspace.

## tcpdump 抓包辅助

抓包文件默认写到 `target/ntx-echo/`，文件名会带时间戳。

Host 侧（`ntx0`）：

```bash
sudo ./examples/ntx-echo/scripts/tcpdump-host.sh
```

Netns 侧（`ntxns1:ntx1`）：

```bash
sudo ./examples/ntx-echo/scripts/tcpdump-netns.sh
```

双向同时抓（推荐，用于定位“ARP reply 插在 UDP 中间”这类时序问题）：

```bash
sudo ./examples/ntx-echo/scripts/tcpdump-bidir.sh
```

它会同时输出两份 pcap 到 `target/ntx-echo/`，方便用时间戳对齐：

- `host-<iface>-<timestamp>.pcap`
- `netns-<ns>-<iface>-<timestamp>.pcap`

默认过滤器是 `arp or (udp and port 7)`，你可以通过环境变量覆盖：

```bash
sudo IFACE=ntx0 FILTER='arp or (udp and port 7)' ./examples/ntx-echo/scripts/tcpdump-host.sh
sudo NS=ntxns1 IFACE=ntx1 FILTER='arp or (udp and port 7)' ./examples/ntx-echo/scripts/tcpdump-netns.sh
```

如果你希望 pcap 里尽量不要出现“本机自己发出去的包”导致的重复帧，可以用 `DIR=in` 只抓入方向（tcpdump 的 `-Q`）：

```bash
sudo DIR=in ./examples/ntx-echo/scripts/tcpdump-host.sh
sudo DIR=in ./examples/ntx-echo/scripts/tcpdump-netns.sh
sudo DIR=in ./examples/ntx-echo/scripts/tcpdump-bidir.sh
```

### sudo + cargo PATH

Some systems configure `sudo` with a restricted `secure_path`, so `cargo` (often installed in `~/.cargo/bin`) may not be found.

If you see `cargo: command not found`, use either:

```bash
sudo -E ./examples/ntx-echo/scripts/run-server.sh
sudo -E ./examples/ntx-echo/scripts/run-client.sh
```

or explicitly pass `CARGO`:

```bash
CARGO=$(command -v cargo) sudo ./examples/ntx-echo/scripts/run-server.sh
CARGO=$(command -v cargo) sudo ./examples/ntx-echo/scripts/run-client.sh
```

### rustup toolchain under sudo

If `sudo ./.../run-*.sh` prints:

> rustup could not choose a version of cargo to run ... no default is configured

That means you’re invoking the `rustup` cargo shim as **root** (HOME=/root), but root has no default toolchain.

The scripts try to use the calling user’s toolchain automatically. You can also force a toolchain:

```bash
sudo TOOLCHAIN=stable ./examples/ntx-echo/scripts/run-server.sh
sudo TOOLCHAIN=stable ./examples/ntx-echo/scripts/run-client.sh
```

Or provide an explicit build command:

```bash
sudo BUILD_CMD="/home/cc/.cargo/bin/cargo" ./examples/ntx-echo/scripts/run-server.sh
sudo BUILD_CMD="/home/cc/.cargo/bin/cargo" ./examples/ntx-echo/scripts/run-client.sh
```
