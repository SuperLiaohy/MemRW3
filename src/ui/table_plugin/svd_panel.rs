use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use eframe::egui::{self, RichText, Ui};

use crate::svd::{SvdField, SvdPeripheral, SvdRegister, SvdTree, load_svd};
use crate::ui::theme;

struct SvdLoadTask {
    receiver: Receiver<Result<(PathBuf, SvdTree), String>>,
}

#[derive(Default)]
pub struct SvdPanelState {
    path: Option<PathBuf>,
    tree: Option<SvdTree>,
    task: Option<SvdLoadTask>,
    error: Option<String>,
    search: String,
    generation: u64,
}

impl SvdPanelState {
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn load_path(&mut self, path: PathBuf) {
        if self.task.is_some() {
            return;
        }
        self.error = None;
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = load_svd(&path)
                .map(|tree| (path, tree))
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        self.task = Some(SvdLoadTask { receiver });
    }

    fn poll(&mut self) {
        let Some(task) = self.task.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("SVD 解析线程意外结束".to_owned()),
        };
        self.task = None;
        match result {
            Ok((path, tree)) => {
                self.path = Some(path);
                self.tree = Some(tree);
                self.error = None;
                self.generation = self.generation.wrapping_add(1);
            }
            Err(error) => self.error = Some(error),
        }
    }
}

pub fn render_svd_panel(ui: &mut Ui, state: &mut SvdPanelState) {
    state.poll();
    if state.task.is_some() {
        ui.ctx().request_repaint_after(Duration::from_millis(50));
    }

    ui.horizontal(|ui| {
        ui.heading(RichText::new("SVD 寄存器").size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(state.task.is_none(), egui::Button::new("加载 SVD"))
                .clicked()
            {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("CMSIS-SVD", &["svd", "xml"])
                    .pick_file()
                {
                    state.load_path(path);
                }
            }
        });
    });

    if let Some(path) = state.path() {
        ui.label(
            RichText::new(path.display().to_string())
                .size(10.0)
                .color(theme::muted_text(ui)),
        );
    }
    ui.add(
        egui::TextEdit::singleline(&mut state.search)
            .hint_text("搜索外设、寄存器或字段...")
            .desired_width(f32::INFINITY),
    );
    if state.task.is_some() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("正在解析 SVD...");
        });
    }
    if let Some(error) = &state.error {
        ui.label(RichText::new(error).color(theme::danger_text(ui)));
    }
    ui.separator();

    let Some(tree) = &state.tree else {
        if state.task.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(35.0);
                ui.label(
                    RichText::new("加载 SVD 文件以浏览外设寄存器").color(theme::muted_text(ui)),
                );
            });
        }
        return;
    };

    ui.label(
        RichText::new(format!(
            "{} · {} 个外设",
            tree.device_name,
            tree.peripherals.len()
        ))
        .size(11.0)
        .color(theme::muted_text(ui)),
    );
    let query = state.search.trim().to_ascii_lowercase();
    egui::ScrollArea::vertical()
        .id_salt(("svd_scroll", state.generation))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (peripheral_index, peripheral) in tree.peripherals.iter().enumerate() {
                if !peripheral_matches(peripheral, &query) {
                    continue;
                }
                let own_match =
                    text_matches(&peripheral.name, peripheral.description.as_deref(), &query);
                let label = format!("{}  @ 0x{:08X}", peripheral.name, peripheral.base_address);
                let response = egui::CollapsingHeader::new(label)
                    .id_salt((state.generation, "peripheral", peripheral_index))
                    .default_open(!query.is_empty())
                    .open((!query.is_empty()).then_some(true))
                    .show(ui, |ui| {
                        for (register_index, register) in peripheral.registers.iter().enumerate() {
                            if !own_match && !register_matches(register, &query) {
                                continue;
                            }
                            render_register(
                                ui,
                                state.generation,
                                peripheral_index,
                                register_index,
                                register,
                                &query,
                            );
                        }
                    });
                if let Some(description) = &peripheral.description {
                    response.header_response.on_hover_text(description);
                }
            }
        });
}

fn render_register(
    ui: &mut Ui,
    generation: u64,
    peripheral_index: usize,
    register_index: usize,
    register: &SvdRegister,
    query: &str,
) {
    let metadata = [
        register.size_bits.map(|size| format!("{size}-bit")),
        register.access.clone(),
        register
            .reset_value
            .map(|value| format!("reset=0x{value:X}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let label = if metadata.is_empty() {
        format!("{}  0x{:08X}", register.name, register.address)
    } else {
        format!("{}  0x{:08X}  {metadata}", register.name, register.address)
    };
    let own_match = text_matches(&register.name, register.description.as_deref(), query);
    let response = egui::CollapsingHeader::new(RichText::new(label).monospace().size(11.0))
        .id_salt((generation, "register", peripheral_index, register_index))
        .default_open(!query.is_empty())
        .open((!query.is_empty()).then_some(true))
        .show(ui, |ui| {
            for field in &register.fields {
                if own_match || field_matches(field, query) {
                    render_field(ui, field);
                }
            }
        });
    if let Some(description) = &register.description {
        response.header_response.on_hover_text(description);
    }
}

fn render_field(ui: &mut Ui, field: &SvdField) {
    let high = field
        .bit_offset
        .saturating_add(field.bit_width.saturating_sub(1));
    let bits = if field.bit_width == 1 {
        format!("[{}]", field.bit_offset)
    } else {
        format!("[{high}:{}]", field.bit_offset)
    };
    let access = field.access.as_deref().unwrap_or("inherited");
    let response = ui.label(
        RichText::new(format!("  {bits:<9} {:<24} {access}", field.name))
            .monospace()
            .size(10.5),
    );
    if let Some(description) = &field.description {
        response.on_hover_text(description);
    }
}

fn peripheral_matches(peripheral: &SvdPeripheral, query: &str) -> bool {
    query.is_empty()
        || text_matches(&peripheral.name, peripheral.description.as_deref(), query)
        || peripheral
            .registers
            .iter()
            .any(|register| register_matches(register, query))
}

fn register_matches(register: &SvdRegister, query: &str) -> bool {
    text_matches(&register.name, register.description.as_deref(), query)
        || register
            .fields
            .iter()
            .any(|field| field_matches(field, query))
}

fn field_matches(field: &SvdField, query: &str) -> bool {
    text_matches(&field.name, field.description.as_deref(), query)
}

fn text_matches(name: &str, description: Option<&str>, query: &str) -> bool {
    query.is_empty()
        || name.to_ascii_lowercase().contains(query)
        || description.is_some_and(|description| description.to_ascii_lowercase().contains(query))
}
