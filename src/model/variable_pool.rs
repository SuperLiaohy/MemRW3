use crate::types::TreeNode;
use std::collections::HashMap;

#[derive(Clone)]
pub struct PooledVariable {
    pub id: usize,
    pub tree_node: TreeNode,
    pub current_value: Vec<u8>,
}

#[derive(Default, Clone)]
pub struct VariablePool {
    variables: Vec<PooledVariable>,
    id_index: HashMap<usize, usize>,
    next_id: usize,
}

impl VariablePool {
    pub fn add(&mut self, node: &TreeNode) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let idx = self.variables.len();
        self.variables.push(PooledVariable {
            id,
            tree_node: node.clone(),
            current_value: Vec::new(),
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

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PooledVariable> {
        self.variables.iter_mut()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.id_index.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.variables.len()
    }
}
