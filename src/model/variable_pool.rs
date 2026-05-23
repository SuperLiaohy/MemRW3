use std::sync::Arc;
use crate::types::{ExtendConfig, ExtendType};
use crate::model::DoubleBuffer;
use std::collections::HashMap;

pub struct PooledVariable {
    pub id: usize,
    pub name: String,
    pub address: u64,
    pub ext_type: ExtendType,
    pub size: u32,
    pub incoming: Arc<DoubleBuffer<(f64, [u8; 8])>>,
    pub plugins_cnt: usize,
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
            incoming: Arc::new(DoubleBuffer::new()),
            plugins_cnt: 0,
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
        self.id_index.get(&id).and_then(|&i| self.variables.get_mut(i))
    }

    pub fn iter(&self) -> impl Iterator<Item = &PooledVariable> {
        self.variables.iter()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.id_index.contains_key(&id)
    }

    pub fn find_by_name_addr(&self, name: &str, address: u64) -> Option<&PooledVariable> {
        self.variables
            .iter()
            .find(|v| v.name == name && v.address == address)
    }
}
