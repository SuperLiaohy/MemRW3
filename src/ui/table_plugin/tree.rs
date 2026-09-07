use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::model::{VariablePool, VariableReadClass};
use crate::ui::plugin::{FrameData, VariableCandidate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    None,
    Partial,
    All,
    Unavailable,
}

pub struct TableLeaf {
    pub variable_id: usize,
    pub enabled: bool,
    pub current_value: String,
    pub edit_buffer: String,
    pub refresh_hz: u32,
    last_value_update: Option<Instant>,
}

pub struct TableNode {
    pub id: u64,
    pub label: String,
    pub source_name: String,
    pub source_address: u64,
    pub expanded: bool,
    pub leaf: Option<TableLeaf>,
    pub children: Vec<TableNode>,
}

impl TableNode {
    pub fn from_candidate(
        candidate: &VariableCandidate,
        root_label: String,
        pool: &mut VariablePool,
        next_id: &mut u64,
    ) -> Result<Self, String> {
        let mut root = Self::build(candidate, pool, next_id);
        if root.leaf_count() == 0 {
            return Err(format!("{} 中没有可读取的标量字段", candidate.name));
        }
        root.label = root_label;
        Ok(root)
    }

    fn build(candidate: &VariableCandidate, pool: &mut VariablePool, next_id: &mut u64) -> Self {
        let id = *next_id;
        *next_id = next_id.saturating_add(1);

        let leaf = candidate.is_readable().then(|| {
            let variable_id = pool
                .find_compatible(
                    &candidate.name,
                    candidate.address,
                    &candidate.ext_type,
                    candidate.size,
                )
                .map(|variable| variable.id)
                .unwrap_or_else(|| pool.add(&candidate.to_config()));
            pool.bind(variable_id, true, VariableReadClass::Latest);
            TableLeaf {
                variable_id,
                enabled: true,
                current_value: "--".to_owned(),
                edit_buffer: String::new(),
                refresh_hz: default_refresh_hz(),
                last_value_update: None,
            }
        });
        let children = candidate
            .children
            .iter()
            .map(|child| Self::build(child, pool, next_id))
            .collect();

        Self {
            id,
            label: candidate.label.clone(),
            source_name: candidate.name.clone(),
            source_address: candidate.address,
            expanded: !candidate.children.is_empty(),
            leaf,
            children,
        }
    }

    pub fn from_saved(
        saved: SavedTableNode,
        pool: &mut VariablePool,
        next_id: &mut u64,
    ) -> Result<Self, String> {
        let id = *next_id;
        *next_id = next_id.saturating_add(1);
        let leaf = match saved.variable {
            Some(saved_leaf) => {
                let variable_id = saved_leaf
                    .variable_type
                    .as_ref()
                    .zip(saved_leaf.variable_size)
                    .and_then(|(ext_type, size)| {
                        pool.find_compatible(
                            &saved_leaf.variable_name,
                            saved_leaf.variable_address,
                            ext_type,
                            size,
                        )
                    })
                    .or_else(|| {
                        pool.find_by_name_addr(
                            &saved_leaf.variable_name,
                            saved_leaf.variable_address,
                        )
                    })
                    .map(|variable| variable.id)
                    .ok_or_else(|| format!("表格变量 \"{}\" 匹配失败", saved_leaf.variable_name))?;
                pool.bind(variable_id, saved_leaf.enabled, VariableReadClass::Latest);
                Some(TableLeaf {
                    variable_id,
                    enabled: saved_leaf.enabled,
                    current_value: "--".to_owned(),
                    edit_buffer: String::new(),
                    refresh_hz: saved_leaf.refresh_hz.clamp(1, 60),
                    last_value_update: None,
                })
            }
            None => None,
        };
        let children = saved
            .children
            .into_iter()
            .map(|child| Self::from_saved(child, pool, next_id))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            id,
            label: saved.label,
            source_name: saved.source_name,
            source_address: saved.source_address,
            expanded: saved.expanded,
            leaf,
            children,
        })
    }

    pub fn to_saved(&self, pool: &VariablePool) -> SavedTableNode {
        let variable = self.leaf.as_ref().and_then(|leaf| {
            pool.get(leaf.variable_id).map(|variable| SavedTableLeaf {
                variable_name: variable.name.clone(),
                variable_address: variable.address,
                variable_type: Some(variable.ext_type.clone()),
                variable_size: Some(variable.size),
                enabled: leaf.enabled,
                refresh_hz: leaf.refresh_hz,
            })
        });
        SavedTableNode {
            label: self.label.clone(),
            source_name: self.source_name.clone(),
            source_address: self.source_address,
            expanded: self.expanded,
            variable,
            children: self
                .children
                .iter()
                .map(|child| child.to_saved(pool))
                .collect(),
        }
    }

    pub fn leaf_count(&self) -> usize {
        usize::from(self.leaf.is_some()) + self.children.iter().map(Self::leaf_count).sum::<usize>()
    }

    pub fn check_state(&self) -> CheckState {
        let (enabled, total) = self.enabled_counts();
        match (enabled, total) {
            (_, 0) => CheckState::Unavailable,
            (0, _) => CheckState::None,
            (enabled, total) if enabled == total => CheckState::All,
            _ => CheckState::Partial,
        }
    }

    fn enabled_counts(&self) -> (usize, usize) {
        let (mut enabled, mut total) = self
            .leaf
            .as_ref()
            .map(|leaf| (usize::from(leaf.enabled), 1))
            .unwrap_or((0, 0));
        for child in &self.children {
            let (child_enabled, child_total) = child.enabled_counts();
            enabled += child_enabled;
            total += child_total;
        }
        (enabled, total)
    }

    pub fn set_enabled_recursive(&mut self, enabled: bool, changes: &mut Vec<(usize, bool)>) {
        if let Some(leaf) = &mut self.leaf {
            if leaf.enabled != enabled {
                leaf.enabled = enabled;
                changes.push((leaf.variable_id, enabled));
            }
        }
        for child in &mut self.children {
            child.set_enabled_recursive(enabled, changes);
        }
    }

    pub fn collect_removals(&self, removals: &mut Vec<(usize, bool)>) {
        if let Some(leaf) = &self.leaf {
            removals.push((leaf.variable_id, leaf.enabled));
        }
        for child in &self.children {
            child.collect_removals(removals);
        }
    }

    pub fn update_values(
        &mut self,
        pool: &VariablePool,
        frame_data: &FrameData,
        formatter: fn(&[u8], &crate::dwarf::types::ExtendType, &mut String),
    ) {
        if let Some(leaf) = &mut self.leaf {
            if leaf.enabled {
                let interval = Duration::from_secs_f64(1.0 / f64::from(leaf.refresh_hz.max(1)));
                let due = leaf
                    .last_value_update
                    .is_none_or(|last_update| last_update.elapsed() >= interval);
                if due {
                    if let Some(variable) = pool.get(leaf.variable_id) {
                        let latest = variable.latest.load().map(|(_, raw)| raw).or_else(|| {
                            frame_data
                                .get(&leaf.variable_id)
                                .and_then(|samples| samples.last())
                                .map(|(_, raw)| *raw)
                        });
                        if let Some(raw) = latest {
                            formatter(&raw, &variable.ext_type, &mut leaf.current_value);
                            leaf.last_value_update = Some(Instant::now());
                        }
                    }
                }
            }
        }
        for child in &mut self.children {
            child.update_values(pool, frame_data, formatter);
        }
    }

    pub fn reset_values(&mut self) {
        if let Some(leaf) = &mut self.leaf {
            leaf.current_value.clear();
            leaf.current_value.push_str("--");
            leaf.last_value_update = None;
        }
        for child in &mut self.children {
            child.reset_values();
        }
    }

    pub fn matches_root(&self, candidate: &VariableCandidate) -> bool {
        self.source_name == candidate.name && self.source_address == candidate.address
    }
}

#[derive(Serialize, Deserialize)]
pub struct SavedTableNode {
    pub label: String,
    pub source_name: String,
    pub source_address: u64,
    #[serde(default = "default_expanded")]
    pub expanded: bool,
    pub variable: Option<SavedTableLeaf>,
    #[serde(default)]
    pub children: Vec<SavedTableNode>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedTableLeaf {
    pub variable_name: String,
    pub variable_address: u64,
    #[serde(default)]
    pub variable_type: Option<crate::dwarf::types::ExtendType>,
    #[serde(default)]
    pub variable_size: Option<u32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_refresh_hz")]
    pub refresh_hz: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_expanded() -> bool {
    true
}

fn default_refresh_hz() -> u32 {
    10
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::dwarf::types::ExtendType;
    use crate::model::VariablePool;
    use crate::ui::plugin::{FrameData, VariableCandidate};

    use super::{CheckState, TableNode};

    fn leaf(name: &str, address: u64) -> VariableCandidate {
        VariableCandidate {
            label: name.to_owned(),
            name: name.to_owned(),
            address,
            ext_type: ExtendType::U32,
            size: 4,
            children: Vec::new(),
        }
    }

    #[test]
    fn parent_check_state_tracks_individual_children() {
        let candidate = VariableCandidate {
            label: "root".to_owned(),
            name: "root".to_owned(),
            address: 0x2000_0000,
            ext_type: ExtendType::Other,
            size: 8,
            children: vec![leaf("root.a", 0x2000_0000), leaf("root.b", 0x2000_0004)],
        };
        let mut pool = VariablePool::default();
        let mut next_id = 0;
        let mut root =
            TableNode::from_candidate(&candidate, "root".to_owned(), &mut pool, &mut next_id)
                .unwrap();

        assert_eq!(root.check_state(), CheckState::All);
        let mut changes = Vec::new();
        root.children[0].set_enabled_recursive(false, &mut changes);
        assert_eq!(root.check_state(), CheckState::Partial);
        root.set_enabled_recursive(false, &mut changes);
        assert_eq!(root.check_state(), CheckState::None);
        assert_eq!(changes.len(), 2);
    }

    #[test]
    fn disabled_leaf_ignores_shared_frame_data() {
        let candidate = leaf("value", 0x2000_0000);
        let mut pool = VariablePool::default();
        let mut next_id = 0;
        let mut root =
            TableNode::from_candidate(&candidate, "value".to_owned(), &mut pool, &mut next_id)
                .unwrap();
        let variable_id = root.leaf.as_ref().unwrap().variable_id;
        let mut frame_data = FrameData::default();
        frame_data.insert(variable_id, vec![(1.0, [7; 8])]);
        root.leaf.as_mut().unwrap().enabled = false;

        root.update_values(&pool, &frame_data, |_, _, output| {
            output.clear();
            output.push_str("updated");
        });
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "--");

        root.leaf.as_mut().unwrap().enabled = true;
        root.update_values(&pool, &frame_data, |_, _, output| {
            output.clear();
            output.push_str("updated");
        });
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "updated");
    }

    #[test]
    fn refresh_rate_throttles_only_the_display_value() {
        let candidate = leaf("value", 0x2000_0000);
        let mut pool = VariablePool::default();
        let mut next_id = 0;
        let mut root =
            TableNode::from_candidate(&candidate, "value".to_owned(), &mut pool, &mut next_id)
                .unwrap();
        let variable_id = root.leaf.as_ref().unwrap().variable_id;
        root.leaf.as_mut().unwrap().refresh_hz = 10;
        let mut frame_data = FrameData::default();
        frame_data.insert(variable_id, vec![(1.0, [1; 8])]);
        let formatter = |raw: &[u8], _: &ExtendType, output: &mut String| {
            *output = raw[0].to_string();
        };

        root.update_values(&pool, &frame_data, formatter);
        frame_data.insert(variable_id, vec![(2.0, [2; 8])]);
        root.update_values(&pool, &frame_data, formatter);
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "1");

        root.leaf.as_mut().unwrap().last_value_update =
            Some(Instant::now() - Duration::from_secs(1));
        root.update_values(&pool, &frame_data, formatter);
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "2");
    }

    #[test]
    fn prefers_latest_value_over_frame_data_and_falls_back_before_first_sample() {
        let candidate = leaf("value", 0x2000_0000);
        let mut pool = VariablePool::default();
        let mut next_id = 0;
        let mut root =
            TableNode::from_candidate(&candidate, "value".to_owned(), &mut pool, &mut next_id)
                .unwrap();
        let variable_id = root.leaf.as_ref().unwrap().variable_id;
        let mut frame_data = FrameData::default();
        frame_data.insert(variable_id, vec![(1.0, [3; 8]), (2.0, [7; 8])]);
        let formatter = |raw: &[u8], _: &ExtendType, output: &mut String| {
            *output = raw[0].to_string();
        };

        root.update_values(&pool, &frame_data, formatter);
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "7");

        pool.get(variable_id).unwrap().latest.store([9; 8]);
        root.leaf.as_mut().unwrap().last_value_update =
            Some(Instant::now() - Duration::from_secs(1));
        root.update_values(&pool, &frame_data, formatter);
        assert_eq!(root.leaf.as_ref().unwrap().current_value, "9");
        assert_eq!(frame_data.get(&variable_id).unwrap().len(), 2);
    }

    #[test]
    fn restores_individual_enabled_states() {
        let first = leaf("root.a", 0x2000_0000);
        let second = leaf("root.b", 0x2000_0004);
        let candidate = VariableCandidate {
            label: "root".to_owned(),
            name: "root".to_owned(),
            address: 0x2000_0000,
            ext_type: ExtendType::Other,
            size: 8,
            children: vec![first.clone(), second.clone()],
        };
        let mut pool = VariablePool::default();
        let mut next_id = 0;
        let mut root =
            TableNode::from_candidate(&candidate, "root".to_owned(), &mut pool, &mut next_id)
                .unwrap();
        root.children[0].leaf.as_mut().unwrap().enabled = false;
        root.children[0].leaf.as_mut().unwrap().refresh_hz = 25;
        let saved = root.to_saved(&pool);

        let mut restored_pool = VariablePool::default();
        restored_pool.add(&first.to_config());
        restored_pool.add(&second.to_config());
        let mut restored_id = 0;
        let restored = TableNode::from_saved(saved, &mut restored_pool, &mut restored_id).unwrap();

        assert_eq!(restored.check_state(), CheckState::Partial);
        assert!(!restored.children[0].leaf.as_ref().unwrap().enabled);
        assert_eq!(restored.children[0].leaf.as_ref().unwrap().refresh_hz, 25);
        assert!(restored.children[1].leaf.as_ref().unwrap().enabled);
        assert_eq!(
            restored_pool
                .iter()
                .map(|variable| variable.active_readers)
                .sum::<usize>(),
            1
        );
    }
}
