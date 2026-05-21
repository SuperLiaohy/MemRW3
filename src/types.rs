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
}
