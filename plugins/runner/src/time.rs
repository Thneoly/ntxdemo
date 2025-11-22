use crate::mywasi::wasi::clocks::monotonic_clock::subscribe_duration;
use crate::runtime::Reactor;
use std::time::Duration;
pub async fn sleep(duration: Duration, reactor: &Reactor) {
    let duration = duration.as_nanos() as u64;
    let pollable = subscribe_duration(duration);
    reactor.wait_for(pollable).await;
}
