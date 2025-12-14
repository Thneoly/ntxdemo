use crate::mywasi::wasi::io::poll::Pollable;
use crate::polling::{EventKey, Poller};
use std::cell::RefCell;
use std::collections::HashMap;
use std::future;
use std::pin::pin;
use std::ptr;
use std::rc::Rc;
use std::sync::OnceLock;
use std::task::Context;
use std::task::Poll;
use std::task::RawWaker;
use std::task::RawWakerVTable;
use std::task::Waker;

// 在单线程WASI环境中，我们可以安全地实现Send和Sync
#[derive(Debug, Clone)]
pub struct Reactor {
    inner: Rc<RefCell<InnerReactor>>,
}

// 为Reactor在单线程环境中实现Send和Sync
unsafe impl Send for Reactor {}
unsafe impl Sync for Reactor {}

#[derive(Debug)]
struct InnerReactor {
    poller: Poller,
    wakers: HashMap<EventKey, Waker>,
}

// Global reactor instance
static GLOBAL_REACTOR: OnceLock<Reactor> = OnceLock::new();

impl Reactor {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(InnerReactor {
                poller: Poller::new(),
                wakers: HashMap::new(),
            })),
        }
    }
    
    /// Get or initialize the global reactor
    pub fn get_global() -> &'static Reactor {
        GLOBAL_REACTOR.get_or_init(|| Reactor::new())
    }
    
    pub async fn wait_for(&self, pollable: Pollable) {
        let mut pollable = Some(pollable);
        let mut key = None;

        future::poll_fn(|cx| {
            let mut reactor = self.inner.borrow_mut();

            let key = key.get_or_insert_with(|| reactor.poller.insert(pollable.take().unwrap()));
            reactor.wakers.insert(*key, cx.waker().clone());

            if reactor.poller.get(key).unwrap().ready() {
                reactor.poller.remove(*key);
                reactor.wakers.remove(key);
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }
    
    pub(crate) fn block_until(&self) {
        let mut reactor = self.inner.borrow_mut();
        for key in reactor.poller.block_until() {
            match reactor.wakers.get(&key) {
                Some(waker) => waker.wake_by_ref(),
                None => panic!("tried to wake the waker for non-existent `{key:?}`"),
            }
        }
    }
}

// Extension trait to create a no-op waker
pub trait WakerExt {
    fn noop() -> Self;
}

impl WakerExt for Waker {
    fn noop() -> Self {
        const VTABLE: RawWakerVTable = RawWakerVTable::new(|_| RAW, |_| {}, |_| {}, |_| {});
        const RAW: RawWaker = RawWaker::new(ptr::null(), &VTABLE);

        unsafe { Waker::from_raw(RAW) }
    }
}

fn noop_waker() -> Waker {
    Waker::noop().clone()
}

pub fn block_on<F, Fut>(f: F) -> Fut::Output
where
    F: FnOnce(&Reactor) -> Fut,
    Fut: Future,
{
    let reactor = Reactor::get_global();
    
    let fut = (f)(reactor);
    let mut fut = pin!(fut);

    let waker = noop_waker();
    let mut cx = Context::from_waker(&waker);

    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(res) => return res,
            Poll::Pending => reactor.block_until(),
        }
    }
}