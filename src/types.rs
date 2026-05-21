use std::cell::RefCell;
use std::collections::HashSet;
use egui_ltreeview::TreeViewState;
use gimli::{UnitOffset, UnitSectionOffset};

#[derive(Debug, Clone)]
pub struct TypeRef {
    pub name: Option<String>,
    pub size: Option<u64>,
    pub kind: TypeKind,
    pub unit_offset: UnitOffset,
    pub unit_header_offset: UnitSectionOffset,
    pub element_type: Option<Box<TypeRef>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    Struct,
    Union,
    Class,
    Other,
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: Option<String>,
    pub offset: u64,
    pub type_ref: TypeRef,
}

pub struct TypeDefInfo {
    pub unit_header_offset: UnitSectionOffset,
    pub unit_offset: UnitOffset,
    pub byte_size: u64,
}

#[derive(Clone)]
pub struct TreeNode {
    pub id: usize,
    pub name: String,
    pub type_name: String,
    pub address_info: String,
    pub size_info: String,
    pub children: Vec<TreeNode>,
}

pub struct CuInfo {
    pub cu_name: String,
    pub variables: Vec<TreeNode>,
    pub dir_id: usize,
}

pub type VisitedKey = (UnitSectionOffset, UnitOffset);

pub struct DwarfApp {
    pub cus: Vec<CuInfo>,
    pub selected_node: Option<TreeNode>,
    pub tree_state: RefCell<TreeViewState<usize>>,
    pub search_text: String,
    pub search_mode: bool,
    pub search_results: HashSet<usize>,
    pub search_path_nodes: HashSet<usize>,
    pub needs_all_reset: bool,
}

impl DwarfApp {
    pub fn new(cus: Vec<CuInfo>) -> Self {
        DwarfApp {
            cus,
            selected_node: None,
            tree_state: RefCell::new(TreeViewState::default()),
            search_text: String::new(),
            search_mode: false,
            search_results: HashSet::new(),
            search_path_nodes: HashSet::new(),
            needs_all_reset: false,
        }
    }

    pub fn perform_search(&mut self) {
        self.search_results.clear();
        self.search_path_nodes.clear();

        let query = self.search_text.trim();
        if query.is_empty() {
            return;
        }

        let levels: Vec<&str> = query.split('.').collect();

        for cu in &self.cus {
            for var in &cu.variables {
                let level0 = &levels[0];
                if !node_name_matches(&var.name, level0) {
                    let results = self.search_in_tree(var, &levels, 0, &mut Vec::new());
                    for path in results {
                        self.search_results
                            .insert(path.last().copied().unwrap_or(var.id));
                        for &id in &path {
                            self.search_path_nodes.insert(id);
                        }
                    }
                    continue;
                }

                if levels.len() == 1 {
                    self.search_results.insert(var.id);
                    self.search_path_nodes.insert(var.id);
                } else {
                    let results =
                        self.search_in_tree(var, &levels, 1, &mut vec![var.id]);
                    for path in results {
                        self.search_results
                            .insert(path.last().copied().unwrap_or(var.id));
                        for &id in &path {
                            self.search_path_nodes.insert(id);
                        }
                    }
                }
            }
        }

        for cu in &self.cus {
            if self.cu_has_result(cu) {
                self.search_path_nodes.insert(cu.dir_id);
            }
        }

        for &id in &self.search_path_nodes {
            self.tree_state.borrow_mut().set_openness(id, true);
        }
        if let Some(&first_id) = self.search_results.iter().next() {
            let all: Vec<usize> = self.search_results.iter().copied().collect();
            self.tree_state.borrow_mut().set_selected(all);
            self.selected_node = self.find_any_node_by_id(first_id);
        }
    }

    fn find_any_node_by_id(&self, id: usize) -> Option<TreeNode> {
        for cu in &self.cus {
            for var in &cu.variables {
                if let Some(node) = self.find_in_subtree(var, id) {
                    return Some(node);
                }
            }
        }
        None
    }

    fn find_in_subtree(&self, node: &TreeNode, id: usize) -> Option<TreeNode> {
        if node.id == id {
            return Some(node.clone());
        }
        for child in &node.children {
            if let Some(found) = self.find_in_subtree(child, id) {
                return Some(found);
            }
        }
        None
    }

    fn search_in_tree(
        &self,
        node: &TreeNode,
        levels: &[&str],
        level_idx: usize,
        path: &mut Vec<usize>,
    ) -> Vec<Vec<usize>> {
        path.push(node.id);
        let mut results = Vec::new();

        if level_idx >= levels.len() {
            results.push(path.clone());
            path.pop();
            return results;
        }

        let target = levels[level_idx];
        for child in &node.children {
            if node_name_matches(&child.name, target) {
                if level_idx + 1 == levels.len() {
                    let mut full_path = path.clone();
                    full_path.push(child.id);
                    results.push(full_path);
                } else {
                    let child_results =
                        self.search_in_tree(child, levels, level_idx + 1, &mut path.clone());
                    results.extend(child_results);
                }
            } else {
                let child_results =
                    self.search_in_tree(child, levels, level_idx, &mut path.clone());
                results.extend(child_results);
            }
        }

        path.pop();
        results
    }

    pub fn find_node_by_id(&self, id: usize) -> Option<TreeNode> {
        for cu in &self.cus {
            for var in &cu.variables {
                if let Some(node) = self.find_in_subtree(var, id) {
                    return Some(node);
                }
            }
        }
        None
    }

    pub fn cu_has_result(&self, cu: &CuInfo) -> bool {
        cu.variables.iter().any(|v| self.subtree_has_result(v))
    }

    fn subtree_has_result(&self, node: &TreeNode) -> bool {
        if self.search_results.contains(&node.id) || self.search_path_nodes.contains(&node.id) {
            return true;
        }
        node.children.iter().any(|c| self.subtree_has_result(c))
    }
}

pub fn node_name_matches(name: &str, query: &str) -> bool {
    name.to_lowercase().contains(&query.to_lowercase())
}
