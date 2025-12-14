/// A wasip2-style select helper macro that returns the index of the
/// first-ready `Pollable` future. Usage:
///
/// let idx = select_index!(expr1, expr2, expr3).await;
/// match idx {
///     0 => { /* expr1 completed */ }
///     1 => { /* expr2 completed */ }
///     2 => { /* expr3 completed */ }
///     _ => unreachable!(),
/// }
///
/// This is a pragmatic, small building block similar to `tokio::select!` but
/// easier to compose with the existing `Reactor::wait_for` API: it accepts a
/// comma-separated list of expressions producing `Pollable`s. It returns the
/// 0-based index of the first-ready pollable.
#[macro_export]
macro_rules! select_index {
    ($($pollable:expr),+ $(,)?) => {{
        {
            async move {
                // Create futures for each pollable
                let mut futures = Vec::new();
                $(
                    futures.push(Box::pin($crate::runtime::Reactor::get_global().wait_for($pollable)));
                )+

                loop {
                    // Create a no-op waker
                    let waker = $crate::runtime::WakerExt::noop();
                    let mut cx = ::std::task::Context::from_waker(&waker);

                    for (index, fut) in futures.iter_mut().enumerate() {
                        if let ::std::task::Poll::Ready(_) = fut.as_mut().poll(&mut cx) {
                            return index;
                        }
                    }

                    // If none are ready, block until one becomes ready
                    $crate::runtime::Reactor::get_global().block_until();
                }
            }
        }
    }};
}