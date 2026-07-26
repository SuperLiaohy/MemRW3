use std::collections::HashMap;

use eframe::egui::{self, Ui};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::VariablePool;

pub type FrameData = HashMap<usize, Vec<(f64, [u8; 8])>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPluginConfig {
    pub plugin_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Success,
    Error,
}

#[derive(Debug)]
pub enum PluginAction {
    OpenVariableTree { plugin_id: String },
    RemoveVariable { var_id: usize },
    WriteVariable { var_id: usize, value: u64 },
    ResetTimer,
    Toast { level: ToastLevel, message: String },
}

pub struct PluginRenderContext<'a> {
    pub pool: &'a VariablePool,
    pub frame_data: &'a FrameData,
    pub running: bool,
}

pub trait MemRWPlugin {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;

    fn viewport_size(&self) -> egui::Vec2 {
        egui::vec2(640.0, 400.0)
    }

    fn min_viewport_size(&self) -> egui::Vec2 {
        egui::vec2(320.0, 220.0)
    }

    fn render(&mut self, ui: &mut Ui, ctx: PluginRenderContext<'_>) -> Vec<PluginAction>;

    fn add_variable_ui(
        &mut self,
        ui: &mut Ui,
        node_id: usize,
        default_name: &str,
        variable_id: usize,
        pool: &VariablePool,
    ) -> bool;

    fn is_dialog_open(&self) -> bool {
        false
    }

    fn save_config(&self, _pool: &VariablePool) -> Value {
        Value::Null
    }

    fn load_config(&mut self, _payload: &Value, _pool: &mut VariablePool) -> Result<(), String> {
        Ok(())
    }
}

pub fn temp_text_value(
    ui: &mut Ui,
    value_id: egui::Id,
    default_id: egui::Id,
    default_name: &str,
) -> String {
    ui.data_mut(|data| {
        let previous_default = data.get_temp::<String>(default_id);
        let mut value = data
            .get_temp::<String>(value_id)
            .unwrap_or_else(|| default_name.to_owned());
        if value.is_empty() || previous_default.as_deref() != Some(default_name) {
            value = default_name.to_owned();
        }
        data.insert_temp(default_id, default_name.to_owned());
        value
    })
}
