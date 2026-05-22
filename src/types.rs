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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Float,
    Double,
    Pointer,
    Struct(String),
    Other(String),
    Array(Box<BasicType>, u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtendType {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    Float,
    Double,
    Other,
}

#[derive(Clone)]
pub struct TreeNode {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub name: String,
    pub struct_name: Option<String>,
    pub type_name: String,
    pub basic_type: BasicType,
    pub address: u64,
    pub size: u32,

    pub children: Vec<TreeNode>,
}

/// Convert BasicType to ExtendType for data acquisition and display.
pub fn basic_type_to_extend(bt: &BasicType) -> ExtendType {
    match bt {
        BasicType::U8 => ExtendType::U8,
        BasicType::U16 => ExtendType::U16,
        BasicType::U32 => ExtendType::U32,
        BasicType::U64 => ExtendType::U64,
        BasicType::I8 => ExtendType::I8,
        BasicType::I16 => ExtendType::I16,
        BasicType::I32 => ExtendType::I32,
        BasicType::I64 => ExtendType::I64,
        BasicType::Float => ExtendType::Float,
        BasicType::Double => ExtendType::Double,
        BasicType::Pointer => ExtendType::U64,
        BasicType::Struct(_) => ExtendType::Other,
        BasicType::Other(_) => ExtendType::Other,
        BasicType::Array(elem, _) => basic_type_to_extend(elem),
    }
}

/// Human-readable label for ExtendType.
pub fn extend_type_label(et: &ExtendType) -> &'static str {
    match et {
        ExtendType::U8 => "u8",
        ExtendType::U16 => "u16",
        ExtendType::U32 => "u32",
        ExtendType::U64 => "u64",
        ExtendType::I8 => "i8",
        ExtendType::I16 => "i16",
        ExtendType::I32 => "i32",
        ExtendType::I64 => "i64",
        ExtendType::Float => "float",
        ExtendType::Double => "double",
        ExtendType::Other => "other",
    }
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
    pub scroll_target_id: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ExtendConfig {
    pub name: String,
    pub address: u64,
    pub ext_type: ExtendType,
    pub size: u32,
    pub array_index: Option<u64>,
    pub array_count: Option<u64>,
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
            needs_all_reset: true,
            scroll_target_id: None,
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
        let total = levels.len();

        for cu in &self.cus {
            for var in &cu.variables {
                if total == 1 {
                    if node_name_matches(&var.name, levels[0]) {
                        self.search_results.insert(var.id);
                        self.search_path_nodes.insert(var.id);
                    }
                } else if node_name_eq(&var.name, levels[0]) {
                    let mut path = vec![var.id];
                    let results = self.search_level(var, &levels, 1, &mut path);
                    for full_path in results {
                        self.search_results.insert(*full_path.last().unwrap());
                        for &id in &full_path {
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
            self.scroll_target_id = Some(first_id);
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

    pub fn parent_array_info(&self, node_id: usize) -> Option<(u64, u64)> {
        let target = self.find_node_by_id(node_id)?;
        let parent_id = target.parent_id?;
        let parent = self.find_node_by_id(parent_id)?;
        if let BasicType::Array(_, count) = parent.basic_type {
            Some((count, target.size as u64))
        } else {
            None
        }
    }

    fn search_level(
        &self,
        node: &TreeNode,
        levels: &[&str],
        level_idx: usize,
        path: &mut Vec<usize>,
    ) -> Vec<Vec<usize>> {
        let is_last = level_idx == levels.len() - 1;
        let target = levels[level_idx];
        let mut results = Vec::new();

        for child in &node.children {
            let matches = if is_last {
                node_name_matches(&child.name, target)
            } else {
                node_name_eq(&child.name, target)
            };

            if matches {
                if is_last {
                    let mut full_path = path.clone();
                    full_path.push(child.id);
                    results.push(full_path);
                } else {
                    let mut child_path = path.clone();
                    child_path.push(child.id);
                    let child_results = self.search_level(child, levels, level_idx + 1, &mut child_path);
                    results.extend(child_results);
                }
            }
        }

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

    pub fn compute_extend_name(&self, node_id: usize) -> String {
        for cu in &self.cus {
            for var in &cu.variables {
                if let Some(path) = find_path_to_node(var, node_id, &var.name) {
                    return path;
                }
            }
        }
        String::new()
    }

    pub fn compute_extend_address(&self, node_id: usize) -> Option<u64> {
        for cu in &self.cus {
            for var in &cu.variables {
                if let Some(addr) = compute_addr_in_tree(var, node_id, var.address, true) {
                    return Some(addr);
                }
            }
        }
        None
    }

    /// Count visible nodes (using tree_state) before the target node.
    /// Works in both search and non-search mode.
    pub fn count_nodes_before(&self, target_id: usize) -> usize {
        let mut count = 0;
        let tree_state = self.tree_state.borrow();
        for cu in &self.cus {
            if cu.variables.is_empty() { continue; }
            if self.search_mode && !self.cu_has_result(cu) { continue; }
            count += 1; // CU dir node
            for var in &cu.variables {
                if Self::count_in_tree_before_static(var, target_id, &mut count, &tree_state) {
                    return count;
                }
            }
        }
        count
    }

    fn count_in_tree_before_static(
        node: &TreeNode,
        target_id: usize,
        count: &mut usize,
        tree_state: &egui_ltreeview::TreeViewState<usize>,
    ) -> bool {
        if node.id == target_id {
            return true;
        }
        *count += 1;
        if node.children.is_empty() {
            return false;
        }
        if tree_state.is_open(&node.id).unwrap_or(false) {
            for child in &node.children {
                if Self::count_in_tree_before_static(child, target_id, count, tree_state) {
                    return true;
                }
            }
        }
        false
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

fn node_name_eq(name: &str, query: &str) -> bool {
    name.eq_ignore_ascii_case(query)
}

fn find_path_to_node(node: &TreeNode, target_id: usize, current_path: &str) -> Option<String> {
    if node.id == target_id {
        return Some(current_path.to_string());
    }
    for child in &node.children {
        let child_path = if child.name.starts_with('[') {
            format!("{}{}", current_path, child.name)
        } else {
            format!("{}.{}", current_path, child.name)
        };
        if let Some(found) = find_path_to_node(child, target_id, &child_path) {
            return Some(found);
        }
    }
    None
}

fn compute_addr_in_tree(node: &TreeNode, target_id: usize, current_addr: u64, is_root: bool) -> Option<u64> {
    let addr = if is_root {
        node.address
    } else {
        current_addr + node.address
    };
    if node.id == target_id {
        return Some(addr);
    }
    for child in &node.children {
        if let Some(found) = compute_addr_in_tree(child, target_id, addr, false) {
            return Some(found);
        }
    }
    None
}
