```shell
cargo build --example userspace-udp-echo
sudo setcap cap_net_raw,cap_net_admin+ep /home/cc/Desktop/code/GitHub/Ntx/target/debug/examples/userspace-udp-echo
/home/cc/Desktop/code/GitHub/Ntx/target/debug/examples/userspace-udp-echo --iface eno1 --port 10001 --verbose
```

```shell
sudo tcpdump -ni wlp10s0 -vv udp port 10001
```

```shell
echo -n 'ping' | nc -u -w1 192.168.31.138 10001
```