use eframe::egui::{self, RichText, Ui};

pub struct TableEntry {
    pub variable_id: usize,
    pub display_name: String,
    pub current_value: String,
    pub edit_buffer: String,
}

impl TableEntry {
    pub fn new(variable_id: usize, display_name: String) -> Self {
        Self {
            variable_id,
            display_name,
            current_value: String::from("--"),
            edit_buffer: String::new(),
        }
    }
}

pub fn table_add_config_ui(ui: &mut Ui, node_name: &str, out_display_name: &mut String) {
    if out_display_name.is_empty() {
        *out_display_name = node_name.to_string();
    }
    ui.horizontal(|ui| {
        ui.label("显示名:");
        ui.text_edit_singleline(out_display_name);
    });
}

pub fn table_entry_dialog_ui(ui: &mut Ui, entry: &mut TableEntry) -> bool {
    let mut remove = false;

    egui::Grid::new("table_entry_dialog")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("显示名:");
            ui.text_edit_singleline(&mut entry.display_name);
            ui.end_row();

            ui.label("当前值:");
            ui.label(&entry.current_value);
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        if ui
            .button(RichText::new("删除").color(egui::Color32::from_rgb(220, 60, 50)))
            .clicked()
        {
            remove = true;
        }
    });

    remove
}
