#[cfg(test)]
mod tests {
    use crate::abr::{Binding, BindingOwner, BindingStore, ResourceView, load_view, store_view};

    #[test]
    fn abr_snapshot_contains_ipv4() {
        let mut store = BindingStore::default();
        store.add(Binding::ipv4_be(0x0a00_0105, BindingOwner::KernelIface)); // 10.0.1.5
        let view = store.snapshot();
        assert!(view.ipv4.contains_be(0x0a00_0105));
        assert!(!view.ipv4.contains_be(0x0a00_0106));
    }

    #[test]
    fn abr_global_swap_is_visible() {
        store_view(ResourceView::empty());
        assert!(!load_view().ipv4.contains_be(0x0a00_0105));

        let mut store = BindingStore::default();
        store.add(Binding::ipv4_be(0x0a00_0105, BindingOwner::KernelIface));
        store_view(store.snapshot());
        assert!(load_view().ipv4.contains_be(0x0a00_0105));
    }
}
