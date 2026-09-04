use crate::dwarf::types::{ExtendConfig, ExtendType};
use crate::model::RingBuffer;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableReadClass {
    Stream,
    Latest,
}

#[derive(Default)]
pub struct LatestValue {
    value: AtomicU64,
    sequence: AtomicU64,
}

impl LatestValue {
    pub fn store(&self, bytes: [u8; 8]) {
        self.value
            .store(u64::from_le_bytes(bytes), Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Release);
    }

    pub fn load(&self) -> Option<(u64, [u8; 8])> {
        let sequence = self.sequence.load(Ordering::Acquire);
        (sequence != 0).then(|| {
            let value = self.value.load(Ordering::Relaxed).to_le_bytes();
            (sequence, value)
        })
    }
}

pub struct PooledVariable {
    pub id: usize,
    pub name: String,
    pub address: u64,
    pub ext_type: ExtendType,
    pub size: u32,
    pub incoming: Arc<RingBuffer<(f64, [u8; 8])>>,
    pub latest: Arc<LatestValue>,
    pub plugins_cnt: usize,
    pub active_readers: usize,
    pub stream_readers: usize,
    pub latest_readers: usize,
}

#[derive(Default)]
pub struct VariablePool {
    variables: Vec<PooledVariable>,
    id_index: HashMap<usize, usize>,
    next_id: usize,
}

impl VariablePool {
    pub fn add(&mut self, config: &ExtendConfig) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let idx = self.variables.len();
        self.variables.push(PooledVariable {
            id,
            name: config.name.clone(),
            address: config.address,
            ext_type: config.ext_type.clone(),
            size: config.size,
            incoming: Arc::new(RingBuffer::new()),
            latest: Arc::new(LatestValue::default()),
            plugins_cnt: 0,
            active_readers: 0,
            stream_readers: 0,
            latest_readers: 0,
        });
        self.id_index.insert(id, idx);
        id
    }

    pub fn remove(&mut self, id: usize) {
        if let Some(&idx) = self.id_index.get(&id) {
            let last = self.variables.len() - 1;
            if idx != last {
                self.variables.swap(idx, last);
                self.id_index.insert(self.variables[idx].id, idx);
            }
            self.variables.pop();
            self.id_index.remove(&id);
        }
    }

    pub fn get(&self, id: usize) -> Option<&PooledVariable> {
        self.id_index.get(&id).and_then(|&i| self.variables.get(i))
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut PooledVariable> {
        self.id_index
            .get(&id)
            .and_then(|&i| self.variables.get_mut(i))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PooledVariable> {
        self.variables.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PooledVariable> {
        self.variables.iter_mut()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.id_index.contains_key(&id)
    }

    pub fn find_by_name_addr(&self, name: &str, address: u64) -> Option<&PooledVariable> {
        self.variables
            .iter()
            .find(|v| v.name == name && v.address == address)
    }

    pub fn find_compatible(
        &self,
        name: &str,
        address: u64,
        ext_type: &ExtendType,
        size: u32,
    ) -> Option<&PooledVariable> {
        self.variables.iter().find(|variable| {
            variable.name == name
                && variable.address == address
                && &variable.ext_type == ext_type
                && variable.size == size
        })
    }

    pub fn bind(&mut self, id: usize, enabled: bool, class: VariableReadClass) -> bool {
        let Some(variable) = self.get_mut(id) else {
            return false;
        };
        variable.plugins_cnt += 1;
        if enabled {
            variable.active_readers += 1;
            match class {
                VariableReadClass::Stream => variable.stream_readers += 1,
                VariableReadClass::Latest => variable.latest_readers += 1,
            }
        }
        true
    }

    pub fn set_binding_enabled(
        &mut self,
        id: usize,
        enabled: bool,
        class: VariableReadClass,
    ) -> bool {
        let Some(variable) = self.get_mut(id) else {
            return false;
        };
        if enabled {
            if variable.active_readers >= variable.plugins_cnt {
                return false;
            }
            variable.active_readers += 1;
            match class {
                VariableReadClass::Stream => variable.stream_readers += 1,
                VariableReadClass::Latest => variable.latest_readers += 1,
            }
        } else {
            if variable.active_readers == 0 {
                return false;
            }
            variable.active_readers -= 1;
            match class {
                VariableReadClass::Stream => {
                    variable.stream_readers = variable.stream_readers.saturating_sub(1)
                }
                VariableReadClass::Latest => {
                    variable.latest_readers = variable.latest_readers.saturating_sub(1)
                }
            }
        }
        true
    }

    pub fn unbind(&mut self, id: usize, was_enabled: bool, class: VariableReadClass) -> bool {
        let should_remove = {
            let Some(variable) = self.get_mut(id) else {
                return false;
            };
            variable.plugins_cnt = variable.plugins_cnt.saturating_sub(1);
            if was_enabled {
                variable.active_readers = variable.active_readers.saturating_sub(1);
                match class {
                    VariableReadClass::Stream => {
                        variable.stream_readers = variable.stream_readers.saturating_sub(1)
                    }
                    VariableReadClass::Latest => {
                        variable.latest_readers = variable.latest_readers.saturating_sub(1)
                    }
                }
            }
            variable.plugins_cnt == 0
        };
        if should_remove {
            self.remove(id);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::dwarf::types::{ExtendConfig, ExtendType};

    use super::{VariablePool, VariableReadClass};

    fn config() -> ExtendConfig {
        ExtendConfig {
            name: "shared".to_owned(),
            address: 0x2000_0000,
            ext_type: ExtendType::U32,
            size: 4,
            array_index: None,
            array_count: None,
        }
    }

    #[test]
    fn tracks_bindings_and_active_readers_independently() {
        let mut pool = VariablePool::default();
        let id = pool.add(&config());
        assert!(pool.bind(id, true, VariableReadClass::Stream));
        assert!(pool.bind(id, true, VariableReadClass::Latest));
        assert_eq!(
            pool.get(id).map(|variable| variable.stream_readers),
            Some(1)
        );
        assert_eq!(
            pool.get(id).map(|variable| variable.latest_readers),
            Some(1)
        );
        assert!(pool.set_binding_enabled(id, false, VariableReadClass::Latest));
        assert_eq!(pool.get(id).map(|variable| variable.plugins_cnt), Some(2));
        assert_eq!(
            pool.get(id).map(|variable| variable.active_readers),
            Some(1)
        );
        assert_eq!(
            pool.get(id).map(|variable| variable.latest_readers),
            Some(0)
        );

        assert!(pool.unbind(id, false, VariableReadClass::Latest));
        assert!(pool.contains(id));
        assert_eq!(
            pool.get(id).map(|variable| variable.active_readers),
            Some(1)
        );

        assert!(pool.unbind(id, true, VariableReadClass::Stream));
        assert!(!pool.contains(id));
    }

    #[test]
    fn latest_value_keeps_table_data_out_of_the_stream_queue() {
        let mut pool = VariablePool::default();
        let id = pool.add(&config());
        let variable = pool.get(id).unwrap();
        variable.latest.store(42_u64.to_le_bytes());

        assert_eq!(variable.latest.load(), Some((1, 42_u64.to_le_bytes())));
        let mut samples = Vec::new();
        assert_eq!(variable.incoming.drain_into(&mut samples), 0);
    }
}
