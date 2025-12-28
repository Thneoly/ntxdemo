```shell
cargo build --example userspace-udp-echo
sudo setcap cap_net_raw,cap_net_admin+ep /home/cc/Desktop/code/GitHub/Ntx/target/debug/examples/userspace-udp-echo
/home/cc/Desktop/code/GitHub/Ntx/target/debug/examples/userspace-udp-echo --iface eno1 --backend afpacket --port 10001 --verbose
```

```shell
sudo tcpdump -ni wlp10s0 -vv udp port 10001
```

```shell
echo -n 'ping' | nc -u -w1 192.168.31.138 10001
```

## Scheduler diagnostics

The scheduler has been hardened so resident tasks (like NicRx) can’t be starved while the run queue is empty:

- **A (default policy): resident-first polling**. Each scheduler loop polls a resident task before any idle waiting.
- **B (safety net): bounded idle wait**. Even if there are no timers and no new submissions, the idle wait is time-bounded so the loop wakes periodically and resident polling continues.

If you still need to prove whether idle waiting is masking RX behavior, you can temporarily disable the scheduler’s idle waiting entirely.

Set in `config/app.yaml`:

- `scheduler.no_idle_wait: true`

Effect:

- The host scheduler will not block on its condvar when idle; it will keep polling resident tasks (e.g. NicRx)
	with a small sleep to avoid pegging CPU.

This is intended for diagnosis only.