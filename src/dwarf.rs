use crate::types::*;
use anyhow::{bail, Context, Result};
use gimli::{
    AttributeValue, DebuggingInformationEntry, Dwarf, EndianSlice, EntriesTreeNode,
    RunTimeEndian, SectionId, Unit, UnitOffset, UnitSectionOffset,
};
use object::{Object, ObjectSection};
use std::collections::{BTreeSet, HashMap};

pub fn load_dwarf<'a>(
    object: &'a object::read::File<'a>,
    endian: RunTimeEndian,
) -> Result<Dwarf<EndianSlice<'a, RunTimeEndian>>> {
    let load_section = |id: SectionId| -> Result<EndianSlice<'a, RunTimeEndian>> {
        let section = object.section_by_name(id.name());
        let data = if let Some(section) = section {
            section
                .data()
                .with_context(|| format!("Failed to read section {}", id.name()))?
        } else {
            &[][..]
        };
        Ok(EndianSlice::new(data, endian))
    };

    let dwarf = Dwarf::load(&load_section)?;
    let has_sections = [
        SectionId::DebugInfo,
        SectionId::DebugAbbrev,
        SectionId::DebugStr,
        SectionId::DebugStrOffsets,
        SectionId::DebugAddr,
        SectionId::DebugRanges,
        SectionId::DebugRngLists,
        SectionId::DebugLoc,
        SectionId::DebugLocLists,
        SectionId::DebugLine,
        SectionId::DebugLineStr,
    ]
    .iter()
    .any(|id| object.section_by_name(id.name()).is_some());
    if !has_sections {
        bail!("ELF file has no DWARF sections");
    }

    Ok(dwarf)
}

pub fn collect_cus(dwarf: &Dwarf<EndianSlice<RunTimeEndian>>) -> Result<Vec<CuInfo>> {
    let mut type_defs: HashMap<String, TypeDefInfo> = HashMap::new();
    let mut units2 = dwarf.units();
    while let Some(header2) = units2.next()? {
        let unit2 = dwarf.unit(header2)?;
        let mut entries = unit2.entries();
        while let Some((_, entry)) = entries.next_dfs()? {
            match entry.tag() {
                gimli::DW_TAG_class_type
                | gimli::DW_TAG_structure_type
                | gimli::DW_TAG_union_type => {
                    if matches!(
                        entry.attr_value(gimli::DW_AT_declaration)?,
                        Some(AttributeValue::Flag(true))
                    ) {
                        continue;
                    }
                    let name = match entry.attr_value(gimli::DW_AT_name)? {
                        Some(attr) => attr_to_string(dwarf, &unit2, attr)?,
                        None => None,
                    };
                    let size = match entry.attr_value(gimli::DW_AT_byte_size)? {
                        Some(attr) => attr_value_to_u64(attr),
                        None => None,
                    };
                    if let (Some(name), Some(size)) = (name, size) {
                        type_defs.insert(
                            name,
                            TypeDefInfo {
                                unit_header_offset: header2.offset(),
                                unit_offset: entry.offset(),
                                byte_size: size,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
    }

    let mut cus = Vec::new();
    let mut next_id: usize = 0;
    let mut units = dwarf.units();
    while let Some(unit_header) = units.next()? {
        let unit = dwarf.unit(unit_header)?;
        let cu_name = {
            let mut tree = unit.entries_tree(None)?;
            let root = tree.root()?;
            let root_entry = root.entry();
            match root_entry.attr_value(gimli::DW_AT_name)? {
                Some(attr) => attr_to_string(dwarf, &unit, attr)?
                    .unwrap_or_else(|| "<unnamed-cu>".to_string()),
                None => "<unnamed-cu>".to_string(),
            }
        };

        next_id += 1;
        let dir_id = next_id;

        let mut variables = Vec::new();
        let mut entries = unit.entries();
        let mut namespace_stack: Vec<(isize, String)> = Vec::new();
        let mut current_depth: isize = 0;
        while let Some((delta_depth, entry)) = entries.next_dfs()? {
            current_depth += delta_depth;
            if delta_depth < 0 {
                while namespace_stack
                    .last()
                    .is_some_and(|(depth, _)| *depth >= current_depth)
                {
                    namespace_stack.pop();
                }
            }

            if entry.tag() == gimli::DW_TAG_namespace {
                let name = match entry.attr_value(gimli::DW_AT_name)? {
                    Some(attr) => attr_to_string(dwarf, &unit, attr)?,
                    None => None,
                };
                if let Some(name) = name {
                    namespace_stack.push((current_depth, name));
                }
            }
            if entry.tag() == gimli::DW_TAG_variable {
                if is_declaration(entry)? {
                    continue;
                }
                let Some(address) = location_address(dwarf, &unit, entry)? else {
                    continue;
                };
                let name = match entry.attr_value(gimli::DW_AT_name)? {
                    Some(attr) => attr_to_string(dwarf, &unit, attr)?,
                    None => None,
                }
                .unwrap_or_else(|| "<unnamed>".to_string());
                let full_name = if namespace_stack.is_empty() {
                    name.clone()
                } else {
                    let namespace = namespace_stack
                        .iter()
                        .map(|(_, name)| name.as_str())
                        .collect::<Vec<_>>()
                        .join("::");
                    format!("{}::{}", namespace, name)
                };

                let Some(type_offset) = entry.attr_value(gimli::DW_AT_type)? else {
                    continue;
                };
                let Some(unit_offset) = type_offset_to_unit_offset(&unit, type_offset)? else {
                    continue;
                };
                let type_ref =
                    resolve_type(dwarf, &unit, unit_offset, unit.header.offset(), &type_defs)?;
                let node = build_variable_node(dwarf, &unit, &full_name, &type_ref, address, &type_defs, &mut next_id)?;
                variables.push(node);
            }
        }

        if !variables.is_empty() {
            cus.push(CuInfo { cu_name, variables, dir_id });
        }
    }

    Ok(cus)
}

pub fn type_offset_to_unit_offset(
    _unit: &Unit<EndianSlice<RunTimeEndian>>,
    value: AttributeValue<EndianSlice<RunTimeEndian>>,
) -> Result<Option<UnitOffset>> {
    match value {
        AttributeValue::UnitRef(offset) => Ok(Some(offset)),
        AttributeValue::DebugInfoRef(_) => Ok(None),
        _ => Ok(None),
    }
}

pub fn build_variable_node(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    variable_name: &str,
    type_ref: &TypeRef,
    address: u64,
    type_defs: &HashMap<String, TypeDefInfo>,
    next_id: &mut usize,
) -> Result<TreeNode> {
    *next_id += 1;
    let my_id = *next_id;
    let type_name = type_ref
        .name
        .clone()
        .unwrap_or_else(|| "<unnamed-type>".to_string());
    let size = type_ref
        .size
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".to_string());
    let mut children = Vec::new();
    if matches!(
        type_ref.kind,
        TypeKind::Struct | TypeKind::Union | TypeKind::Class
    ) {
        let fields = struct_fields(dwarf, type_ref, &type_defs)?;
        let mut visited = BTreeSet::new();
        let key = (type_ref.unit_header_offset, type_ref.unit_offset);
        visited.insert(key);
        for field in fields {
            children.push(build_field_node(dwarf, unit, &field, &mut visited, &type_defs, next_id)?);
        }
    }
    if let Some(elem) = type_ref.element_type.as_deref() {
        let elem_type_name = elem.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
        let elem_size = elem
            .size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "?".to_string());
        let mut elem_children = Vec::new();
        if matches!(
            elem.kind,
            TypeKind::Struct | TypeKind::Union | TypeKind::Class
        ) {
            let fields = struct_fields(dwarf, elem, &type_defs)?;
            let mut visited = BTreeSet::new();
            let key = (elem.unit_header_offset, elem.unit_offset);
            visited.insert(key);
            for field in fields {
                elem_children.push(build_field_node(dwarf, unit, &field, &mut visited, &type_defs, next_id)?);
            }
        }
        *next_id += 1;
        let elem_child_id = *next_id;
        children.push(TreeNode {
            id: elem_child_id,
            name: format!("element_type: {}", elem_type_name),
            type_name: elem_type_name,
            address_info: String::new(),
            size_info: format!("(size: {})", elem_size),
            children: elem_children,
        });
    }

    Ok(TreeNode {
        id: my_id,
        name: variable_name.to_string(),
        type_name,
        address_info: format!("@ {}", format_address(address)),
        size_info: format!("(size: {})", size),
        children,
    })
}

pub fn build_field_node(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    field: &FieldInfo,
    visited: &mut BTreeSet<VisitedKey>,
    type_defs: &HashMap<String, TypeDefInfo>,
    next_id: &mut usize,
) -> Result<TreeNode> {
    *next_id += 1;
    let my_id = *next_id;
    let name = field
        .name
        .clone()
        .unwrap_or_else(|| "<unnamed>".to_string());
    let type_name = field
        .type_ref
        .name
        .clone()
        .unwrap_or_else(|| "<unnamed-type>".to_string());
    let size = field
        .type_ref
        .size
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".to_string());

    let mut children = Vec::new();
    if matches!(
        field.type_ref.kind,
        TypeKind::Struct | TypeKind::Union | TypeKind::Class
    ) {
        let key = (
            field.type_ref.unit_header_offset,
            field.type_ref.unit_offset,
        );
        if !visited.contains(&key) {
            visited.insert(key);
            let nested_fields = struct_fields(dwarf, &field.type_ref, type_defs)?;
            for nested in nested_fields {
                children.push(build_field_node(dwarf, unit, &nested, visited, type_defs, next_id)?);
            }
        }
    }
    if let Some(elem) = field.type_ref.element_type.as_deref() {
        let elem_type_name = elem.name.clone().unwrap_or_else(|| "<unnamed>".to_string());
        let elem_size = elem
            .size
            .map(|s| s.to_string())
            .unwrap_or_else(|| "?".to_string());
        let mut elem_children = Vec::new();
        if matches!(
            elem.kind,
            TypeKind::Struct | TypeKind::Union | TypeKind::Class
        ) {
            let key = (elem.unit_header_offset, elem.unit_offset);
            if !visited.contains(&key) {
                visited.insert(key);
                let nested_fields = struct_fields(dwarf, elem, type_defs)?;
                for nested in nested_fields {
                    elem_children.push(build_field_node(dwarf, unit, &nested, visited, type_defs, next_id)?);
                }
            }
        }
        *next_id += 1;
        let elem_child_id = *next_id;
        children.push(TreeNode {
            id: elem_child_id,
            name: format!("element_type: {}", elem_type_name),
            type_name: elem_type_name,
            address_info: String::new(),
            size_info: format!("(size: {})", elem_size),
            children: elem_children,
        });
    }

    Ok(TreeNode {
        id: my_id,
        name,
        type_name,
        address_info: format!("@ offset {}", field.offset),
        size_info: format!("(size: {})", size),
        children,
    })
}

pub fn resolve_type(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    offset: UnitOffset,
    unit_header_offset: UnitSectionOffset,
    type_defs: &HashMap<String, TypeDefInfo>,
) -> Result<TypeRef> {
    resolve_type_impl(dwarf, unit, offset, unit_header_offset, None, type_defs)
}

pub fn resolve_type_impl(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    offset: UnitOffset,
    unit_header_offset: UnitSectionOffset,
    outer_name: Option<String>,
    type_defs: &HashMap<String, TypeDefInfo>,
) -> Result<TypeRef> {
    let entry = unit.entry(offset)?;

    match entry.tag() {
        // Follow typedef: preserve the alias name, recurse for size/kind
        gimli::DW_TAG_typedef => {
            let typedef_name = match entry.attr_value(gimli::DW_AT_name)? {
                Some(attr) => attr_to_string(dwarf, unit, attr)?,
                None => None,
            };
            // Prefer the outermost typedef name
            let effective_name = typedef_name.or(outer_name);
            if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                    return resolve_type_impl(
                        dwarf,
                        unit,
                        next,
                        unit_header_offset,
                        effective_name,
                        type_defs,
                    );
                }
            }
            return Ok(TypeRef {
                name: effective_name.or_else(|| Some("<unnamed-typedef>".to_string())),
                size: None,
                kind: TypeKind::Other,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            });
        }

        // Pointer type: "element_type *"
        gimli::DW_TAG_pointer_type => {
            let size = entry
                .attr_value(gimli::DW_AT_byte_size)?
                .and_then(|a| attr_value_to_u64(a))
                .or(Some(u64::from(unit.header.address_size())));
            let pointed_name = if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                    let inner =
                        resolve_type_impl(dwarf, unit, next, unit_header_offset, None, type_defs)?;
                    inner.name.unwrap_or_else(|| "<unnamed>".to_string())
                } else {
                    "<unnamed>".to_string()
                }
            } else {
                "void".to_string()
            };
            Ok(TypeRef {
                name: Some(format!("{} *", pointed_name)),
                size: size.or(Some(u64::from(unit.header.address_size()))),
                kind: TypeKind::Other,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }

        // Const / volatile: transparent pass-through
        gimli::DW_TAG_const_type | gimli::DW_TAG_volatile_type => {
            if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                    return resolve_type_impl(
                        dwarf,
                        unit,
                        next,
                        unit_header_offset,
                        outer_name,
                        type_defs,
                    );
                }
            }
            Ok(TypeRef {
                name: outer_name.or_else(|| Some("<unnamed>".to_string())),
                size: None,
                kind: TypeKind::Other,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }

        // Reference type: "T &"
        gimli::DW_TAG_reference_type => {
            let inner = if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                    resolve_type_impl(dwarf, unit, next, unit_header_offset, outer_name, type_defs)?
                } else {
                    TypeRef {
                        name: outer_name.or_else(|| Some("<unnamed>".to_string())),
                        size: None,
                        kind: TypeKind::Other,
                        unit_offset: offset,
                        unit_header_offset,
                        element_type: None,
                    }
                }
            } else {
                TypeRef {
                    name: outer_name.or_else(|| Some("<unnamed>".to_string())),
                    size: None,
                    kind: TypeKind::Other,
                    unit_offset: offset,
                    unit_header_offset,
                    element_type: None,
                }
            };
            let inner_name = inner
                .name
                .clone()
                .unwrap_or_else(|| "<unnamed>".to_string());
            Ok(TypeRef {
                name: Some(format!("{} &", inner_name)),
                size: Some(u64::from(unit.header.address_size())),
                kind: inner.kind,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }

        // Rvalue reference type: "T &&"
        gimli::DW_TAG_rvalue_reference_type => {
            let inner = if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                    resolve_type_impl(dwarf, unit, next, unit_header_offset, outer_name, type_defs)?
                } else {
                    TypeRef {
                        name: outer_name.or_else(|| Some("<unnamed>".to_string())),
                        size: None,
                        kind: TypeKind::Other,
                        unit_offset: offset,
                        unit_header_offset,
                        element_type: None,
                    }
                }
            } else {
                TypeRef {
                    name: outer_name.or_else(|| Some("<unnamed>".to_string())),
                    size: None,
                    kind: TypeKind::Other,
                    unit_offset: offset,
                    unit_header_offset,
                    element_type: None,
                }
            };
            let inner_name = inner
                .name
                .clone()
                .unwrap_or_else(|| "<unnamed>".to_string());
            Ok(TypeRef {
                name: Some(format!("{} &&", inner_name)),
                size: Some(u64::from(unit.header.address_size())),
                kind: inner.kind,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }

        // Array type: construct "uint8_t[N]" name
        gimli::DW_TAG_array_type => {
            let (element_type_ref, elem_name, elem_size) =
                if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                    if let Some(next) = type_offset_to_unit_offset(unit, attr)? {
                        let inner = resolve_type_impl(
                            dwarf,
                            unit,
                            next,
                            unit_header_offset,
                            None,
                            type_defs,
                        )?;
                        let elem_name = inner
                            .name
                            .clone()
                            .unwrap_or_else(|| "<unnamed>".to_string());
                        let elem_size = inner.size;
                        (Some(inner), elem_name, elem_size)
                    } else {
                        (None, "<unnamed>".to_string(), None)
                    }
                } else {
                    (None, "<unnamed>".to_string(), None)
                };

            // Read subrange children for array dimensions
            let mut dims = Vec::new();
            let mut total_size = elem_size.unwrap_or(0);
            // Use a ref instead of moving offset
            let array_offset = offset;
            let mut tree = unit.entries_tree(Some(array_offset))?;
            let root = tree.root()?;
            let mut children = root.children();
            while let Some(child_node) = children.next()? {
                let child = child_node.entry();
                if child.tag() == gimli::DW_TAG_subrange_type {
                    let count = match child.attr_value(gimli::DW_AT_upper_bound)? {
                        Some(attr) => attr_value_to_u64(attr).map(|v| v + 1),
                        None => child
                            .attr_value(gimli::DW_AT_count)?
                            .and_then(|a| attr_value_to_u64(a)),
                    };
                    if let Some(c) = count {
                        dims.push(c);
                        total_size *= c;
                    }
                }
            }

            let array_name = if dims.is_empty() {
                format!("{}[]", elem_name)
            } else {
                let dims_str = dims
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("][");
                format!("{}[{}]", elem_name, dims_str)
            };
            Ok(TypeRef {
                name: Some(array_name),
                size: Some(total_size),
                kind: TypeKind::Other,
                unit_offset: offset,
                unit_header_offset,
                element_type: element_type_ref.map(Box::new),
            })
        }

        // Struct / union / class: read name, size, kind. Use outer typedef name if inner is unnamed.
        gimli::DW_TAG_structure_type
        | gimli::DW_TAG_union_type
        | gimli::DW_TAG_class_type
        | gimli::DW_TAG_enumeration_type => {
            let is_decl = matches!(
                entry.attr_value(gimli::DW_AT_declaration)?,
                Some(AttributeValue::Flag(true))
            );
            if is_decl {
                let inner_name = match entry.attr_value(gimli::DW_AT_name)? {
                    Some(attr) => attr_to_string(dwarf, unit, attr)?,
                    None => None,
                };
                if let Some(def_name) = &inner_name {
                    if let Some(def) = type_defs.get(def_name) {
                        let name = inner_name.or(outer_name);
                        return Ok(TypeRef {
                            name,
                            size: Some(def.byte_size),
                            kind: match entry.tag() {
                                gimli::DW_TAG_structure_type => TypeKind::Struct,
                                gimli::DW_TAG_union_type => TypeKind::Union,
                                gimli::DW_TAG_class_type => TypeKind::Class,
                                _ => TypeKind::Other,
                            },
                            unit_offset: def.unit_offset,
                            unit_header_offset: def.unit_header_offset,
                            element_type: None,
                        });
                    }
                }
            }
            let inner_name = match entry.attr_value(gimli::DW_AT_name)? {
                Some(attr) => attr_to_string(dwarf, unit, attr)?,
                None => None,
            };
            let name = inner_name.or(outer_name);

            let size = {
                let attr = entry.attr_value(gimli::DW_AT_byte_size)?;
                attr.and_then(attr_value_to_u64)
            };
            let kind = match entry.tag() {
                gimli::DW_TAG_structure_type => TypeKind::Struct,
                gimli::DW_TAG_union_type => TypeKind::Union,
                gimli::DW_TAG_class_type => TypeKind::Class,
                _ => TypeKind::Other,
            };
            Ok(TypeRef {
                name,
                size,
                kind,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }

        // Base type (int, float, char, ...) or anything else
        _ => {
            let name = match entry.attr_value(gimli::DW_AT_name)? {
                Some(attr) => attr_to_string(dwarf, unit, attr)?,
                None => None,
            }
            .or(outer_name);
            let size = match entry.attr_value(gimli::DW_AT_byte_size)? {
                Some(attr) => attr_value_to_u64(attr),
                None => None,
            };
            let kind = match entry.tag() {
                gimli::DW_TAG_structure_type => TypeKind::Struct,
                gimli::DW_TAG_union_type => TypeKind::Union,
                gimli::DW_TAG_class_type => TypeKind::Class,
                _ => TypeKind::Other,
            };
            Ok(TypeRef {
                name,
                size,
                kind,
                unit_offset: offset,
                unit_header_offset,
                element_type: None,
            })
        }
    }
}

pub fn attr_value_to_u64(attr: AttributeValue<EndianSlice<RunTimeEndian>>) -> Option<u64> {
    match attr {
        AttributeValue::Udata(v) => Some(v),
        AttributeValue::Data1(v) => Some(v as u64),
        AttributeValue::Data2(v) => Some(v as u64),
        AttributeValue::Data4(v) => Some(v as u64),
        AttributeValue::Data8(v) => Some(v),
        _ => None,
    }
}

pub fn location_address(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    entry: &DebuggingInformationEntry<EndianSlice<RunTimeEndian>>,
) -> Result<Option<u64>> {
    let Some(attr) = entry.attr_value(gimli::DW_AT_location)? else {
        return Ok(None);
    };
    let expr = match attr {
        AttributeValue::Exprloc(expr) => expr,
        AttributeValue::LocationListsRef(offset) => {
            let mut locs = dwarf.locations(unit, offset)?;
            match locs.next()? {
                Some(loc) => loc.data,
                None => return Ok(None),
            }
        }
        _ => return Ok(None),
    };

    let mut ops = expr.operations(unit.encoding());
    let Some(op) = ops.next()? else {
        return Ok(None);
    };
    let address = match op {
        gimli::Operation::Address { address } => Some(address),
        gimli::Operation::AddressIndex { index } => {
            let addr = dwarf.debug_addr.get_address(
                unit.encoding().address_size,
                unit.addr_base,
                index,
            )?;
            Some(addr)
        }
        _ => None,
    };

    if ops.next()?.is_some() {
        return Ok(None);
    }

    Ok(address)
}

pub fn is_declaration(entry: &DebuggingInformationEntry<EndianSlice<RunTimeEndian>>) -> Result<bool> {
    match entry.attr_value(gimli::DW_AT_declaration)? {
        Some(AttributeValue::Flag(true)) => Ok(true),
        _ => Ok(false),
    }
}

pub fn struct_fields(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    type_ref: &TypeRef,
    type_defs: &HashMap<String, TypeDefInfo>,
) -> Result<Vec<FieldInfo>> {
    let mut units = dwarf.units();
    while let Some(unit_header) = units.next()? {
        let unit = dwarf.unit(unit_header)?;
        if unit.header.offset() != type_ref.unit_header_offset {
            continue;
        }
        let mut tree = unit.entries_tree(Some(type_ref.unit_offset))?;
        let root = tree.root()?;
        return collect_fields(dwarf, &unit, root, type_defs);
    }
    Ok(Vec::new())
}

pub fn collect_fields(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    node: EntriesTreeNode<EndianSlice<RunTimeEndian>>,
    type_defs: &HashMap<String, TypeDefInfo>,
) -> Result<Vec<FieldInfo>> {
    let mut fields = Vec::new();
    let mut children = node.children();
    while let Some(child) = children.next()? {
        let entry = child.entry();
        if entry.tag() == gimli::DW_TAG_member {
            let name = match entry.attr_value(gimli::DW_AT_name)? {
                Some(attr) => attr_to_string(dwarf, unit, attr)?,
                None => None,
            };
            let offset = member_offset(unit, entry)?;
            let type_ref = if let Some(attr) = entry.attr_value(gimli::DW_AT_type)? {
                if let Some(unit_offset) = type_offset_to_unit_offset(unit, attr)? {
                    resolve_type(dwarf, unit, unit_offset, unit.header.offset(), type_defs)?
                } else {
                    TypeRef {
                        name: None,
                        size: None,
                        kind: TypeKind::Other,
                        unit_offset: entry.offset(),
                        unit_header_offset: unit.header.offset(),
                        element_type: None,
                    }
                }
            } else {
                TypeRef {
                    name: None,
                    size: None,
                    kind: TypeKind::Other,
                    unit_offset: entry.offset(),
                    unit_header_offset: unit.header.offset(),
                    element_type: None,
                }
            };
            fields.push(FieldInfo {
                name,
                offset,
                type_ref,
            });
        }
    }
    fields.sort_by_key(|field| field.offset);
    Ok(fields)
}

pub fn member_offset(
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    entry: &DebuggingInformationEntry<EndianSlice<RunTimeEndian>>,
) -> Result<u64> {
    match entry.attr_value(gimli::DW_AT_data_member_location)? {
        Some(AttributeValue::Udata(value)) => Ok(value),
        Some(AttributeValue::Data1(value)) => Ok(value as u64),
        Some(AttributeValue::Data2(value)) => Ok(value as u64),
        Some(AttributeValue::Data4(value)) => Ok(value as u64),
        Some(AttributeValue::Data8(value)) => Ok(value),
        Some(AttributeValue::Exprloc(expr)) => {
            let mut ops = expr.operations(unit.encoding());
            if let Some(op) = ops.next()? {
                if let gimli::Operation::UnsignedConstant { value } = op {
                    return Ok(value);
                }
            }
            Ok(0)
        }
        _ => Ok(0),
    }
}

pub fn format_address(address: u64) -> String {
    format!("0x{address:08x}")
}

pub fn attr_to_string(
    dwarf: &Dwarf<EndianSlice<RunTimeEndian>>,
    unit: &Unit<EndianSlice<RunTimeEndian>>,
    attr: AttributeValue<EndianSlice<RunTimeEndian>>,
) -> Result<Option<String>> {
    // dwarf.attr_string() handles all DWARF versions: DW_FORM_strp, DW_FORM_strx*, DW_FORM_string
    match dwarf.attr_string(unit, attr) {
        Ok(s) => {
            let cow = s.to_string_lossy();
            Ok(Some(cow.into_owned()))
        }
        Err(_) => Ok(None),
    }
}
