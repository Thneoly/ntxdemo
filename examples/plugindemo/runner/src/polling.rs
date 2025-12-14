use crate::mywasi::wasi::io::poll::Pollable;
use crate::mywasi::wasi::io::poll::poll;
use slab::Slab;

#[derive(Debug)]
pub(crate) struct Poller {
    pub(crate) targets: Slab<Pollable>,
}
#[repr(transparent)]
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy)]
pub(crate) struct EventKey(pub(crate) u32);

impl Poller {
    pub(crate) fn new() -> Self {
        Self {
            targets: Slab::new(),
        }
    }

    pub(crate) fn insert(&mut self, target: Pollable) -> EventKey {
        let key = self.targets.insert(target);
        EventKey(key as u32)
    }

    pub(crate) fn get(&self, key: &EventKey) -> Option<&Pollable> {
        self.targets.get(key.0 as usize)
    }

    pub(crate) fn remove(&mut self, key: EventKey) -> Option<Pollable> {
        self.targets.try_remove(key.0 as usize)
    }

    pub fn block_until(&mut self) -> Vec<EventKey> {
        // If there are no targets, return an empty vector
        if self.targets.is_empty() {
            return vec![];
        }
        
        let mut indexes = vec![];
        let mut targets = vec![];

        for (index, target) in self.targets.iter() {
            indexes.push(index);
            targets.push(target);
        }

        let ready_indexs = poll(&targets);

        ready_indexs
            .into_iter()
            .map(|index| EventKey(indexes[index as usize] as u32))
            .collect()
    }
}