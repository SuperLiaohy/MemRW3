use eframe::egui::{self, RichText, Ui};
use crate::types::TreeNode;

pub fn vari_properties_ui(
    ui: &mut Ui,
    node: &TreeNode,
    add_config_ui: impl FnOnce(&mut Ui, &str) -> bool,
) -> bool {
    ui.heading("属性");
    ui.separator();

    egui::Grid::new("vari_props")
        .striped(true)
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(RichText::new("Name:").size(12.0));
            ui.label(RichText::new(&node.name).size(12.0));
            ui.end_row();
            ui.label(RichText::new("Type:").size(12.0));
            ui.label(RichText::new(&node.type_name).size(12.0));
            ui.end_row();
            ui.label(RichText::new("Address:").size(12.0));
            ui.label(RichText::new(&node.address_info).size(12.0));
            ui.end_row();
            ui.label(RichText::new("Size:").size(12.0));
            ui.label(RichText::new(&node.size_info).size(12.0));
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    ui.label(RichText::new("添加配置").size(13.0).strong());
    ui.add_space(4.0);

    add_config_ui(ui, &node.name)
}
