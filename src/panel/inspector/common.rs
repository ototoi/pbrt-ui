use std::vec;

use super::resource_selector::ResourceSelector;
use crate::model::base::*;
use crate::model::scene::*;

use eframe::egui;
use eframe::egui::Checkbox;
use eframe::egui::Widget;
use egui_extras;
use uuid::Uuid;

pub const MIN_COMPONENT_HEIGHT: f32 = 100.0;

fn get_label3(key_type: &str) -> (String, String, String) {
    match key_type {
        "color" | "rgb" => ("R".to_string(), "G".to_string(), "B".to_string()),
        _ => ("X".to_string(), "Y".to_string(), "Z".to_string()),
    }
}

#[inline]
pub fn gamma_correct(value: f32) -> f32 {
    if value <= 0.0031308 {
        return 12.92 * value;
    } else {
        return 1.055 * f32::powf(value, 1.0 / 2.4) - 0.055;
    }
}

#[inline]
pub fn inverse_gamma_correct(value: f32) -> f32 {
    if value <= 0.04045 {
        return value * 1.0 / 12.92;
    } else {
        return f32::powf((value + 0.055) * 1.0 / 1.055, 2.4);
    }
}

#[inline]
fn to_byte(v: f32) -> u8 {
    f32::clamp(255.0 * gamma_correct(v), 0.0, 255.0) as u8
}

#[inline]
fn from_byte(v: u8) -> f32 {
    inverse_gamma_correct(v as f32 / 255.0)
}

fn xyz_to_rgb(xyz: &[f32]) -> [f32; 3] {
    let mut rgb: [f32; 3] = [0.0; 3];
    rgb[0] = 3.240479 * xyz[0] - 1.537150 * xyz[1] - 0.498535 * xyz[2];
    rgb[1] = -0.969256 * xyz[0] + 1.875991 * xyz[1] + 0.041556 * xyz[2];
    rgb[2] = 0.055648 * xyz[0] - 0.204043 * xyz[1] + 1.057311 * xyz[2];
    return rgb;
}

fn rgb_to_xyz(rgb: &[f32]) -> [f32; 3] {
    let mut xyz: [f32; 3] = [0.0; 3];
    xyz[0] = 0.412453 * rgb[0] + 0.357580 * rgb[1] + 0.180423 * rgb[2];
    xyz[1] = 0.212671 * rgb[0] + 0.715160 * rgb[1] + 0.072169 * rgb[2];
    xyz[2] = 0.019334 * rgb[0] + 0.119193 * rgb[1] + 0.950227 * rgb[2];
    return xyz;
}

fn get_intensity(value: &[f32]) -> (f32, Vec<f32>) {
    let intensity = value.iter().fold(1.0f32, |acc, &x| acc.max(x));
    let new_value = value.iter().map(|&x| x / intensity).collect::<Vec<f32>>();
    return (intensity, new_value);
}

fn show_rgb(ui: &mut egui::Ui, value: &mut [f32]) -> bool {
    let mut is_changed = false;
    if value.len() >= 3 {
        let (mut intensity, new_value) = get_intensity(value);
        let mut new_value: [f32; 3] = new_value.try_into().unwrap();
        if ui.color_edit_button_rgb(&mut new_value).changed() {
            is_changed = true;
        }
        if ui
            .add(egui::widgets::Slider::new(&mut intensity, 1.0..=100.0))
            .changed()
        {
            is_changed = true;
        }
        value[0] = new_value[0] * intensity;
        value[1] = new_value[1] * intensity;
        value[2] = new_value[2] * intensity;
    }
    return is_changed;
}

fn show_floats(
    ui: &mut egui::Ui,
    key_type: &str,
    _key_name: &str,
    range: &Option<ValueRange>,
    value: &mut Vec<f32>,
) -> bool {
    let mut is_changed = false;
    if value.len() == 3 {
        let (x_label, y_label, z_label) = get_label3(key_type);
        ui.horizontal(|ui| {
            ui.label(&x_label);
            if ui
                .add(egui::widgets::DragValue::new(&mut value[0]))
                .changed()
            {
                is_changed = true;
            }
            ui.label(&y_label);
            if ui
                .add(egui::widgets::DragValue::new(&mut value[1]))
                .changed()
            {
                is_changed = true;
            }
            ui.label(&z_label);
            if ui
                .add(egui::widgets::DragValue::new(&mut value[2]))
                .changed()
            {
                is_changed = true;
            }
        });
    } else if value.len() == 2 {
        let (x_label, y_label, _z_label) = get_label3(key_type);
        ui.horizontal(|ui| {
            ui.label(&x_label);
            if ui
                .add(egui::widgets::DragValue::new(&mut value[0]))
                .changed()
            {
                is_changed = true;
            }
            ui.label(&y_label);
            if ui
                .add(egui::widgets::DragValue::new(&mut value[1]))
                .changed()
            {
                is_changed = true;
            }
        });
    } else if value.len() == 1 {
        if let Some(r) = range {
            if let ValueRange::FloatRange(min, max) = r {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::widgets::Slider::new(&mut value[0], *min..=*max).step_by(0.01))
                        .changed()
                    {
                        is_changed = true;
                    }
                });
            }
        } else {
            ui.horizontal(|ui| {
                if ui
                    .add(egui::widgets::DragValue::new(&mut value[0]))
                    .changed()
                {
                    is_changed = true;
                }
            });
        }
    } else if value.len() >= 4 {
        ui.horizontal(|ui| {
            for i in 0..value.len() {
                if ui
                    .add(egui::widgets::DragValue::new(&mut value[i]))
                    .changed()
                {
                    is_changed = true;
                }
            }
        });
    }
    return is_changed;
}

fn show_ints(
    ui: &mut egui::Ui,
    key_type: &str,
    key_name: &str,
    range: &Option<ValueRange>,
    value: &mut Vec<i32>,
) -> bool {
    let mut is_changed = false;
    if value.len() == 1 {
        let minus = egui::RichText::new("-").family(egui::FontFamily::Monospace);
        let plus = egui::RichText::new("+").family(egui::FontFamily::Monospace);
        if let Some(r) = range {
            if let ValueRange::IntRange(min, max) = r {
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::widgets::DragValue::new(&mut value[0]).range(*min..=*max))
                        .changed()
                    {
                        is_changed = true;
                    }
                    if ui.small_button(minus).clicked() {
                        value[0] -= 1;
                        is_changed = true;
                    }
                    if ui.small_button(plus).clicked() {
                        value[0] += 1;
                        is_changed = true;
                    }
                });
            }
        } else {
            ui.horizontal(|ui| {
                if ui
                    .add(egui::widgets::DragValue::new(&mut value[0]))
                    .changed()
                {
                    is_changed = true;
                }
                if ui.small_button(minus).clicked() {
                    value[0] -= 1;
                    is_changed = true;
                }
                if ui.small_button(plus).clicked() {
                    value[0] += 1;
                    is_changed = true;
                }
            });
        }
    }
    return is_changed;
}

fn show_strings(
    ui: &mut egui::Ui,
    key_type: &str,
    key_name: &str,
    value: &mut Vec<String>,
    resource_selector: &ResourceSelector,
    own_id: Option<Uuid>,
) -> bool {
    let mut is_changed = false;
    if value.len() >= 1 {
        if key_name == "splitmethod" {
            let types = vec!["sah", "hlbvh", "middle", "equal"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();
            egui::ComboBox::from_id_salt("splitmethod")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "strategy" {
            let types = vec!["all", "one"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();
            egui::ComboBox::from_id_salt("strategy")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "lightsamplestrategy" {
            let types = vec!["uniform", "power", "spatial"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();
            egui::ComboBox::from_id_salt("lightsamplestrategy")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "wrap" {
            let types = vec!["repeat", "black", "clamp"] //mirror
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();
            egui::ComboBox::from_id_salt("wrap")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "mapping" {
            let types = vec!["uv", "spherical", "cylindrical", "planar"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<String>>();
            egui::ComboBox::from_id_salt("mapping")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "bumpmap" {
            let mut items = vec![(Uuid::default(), "".to_string(), "none".to_string())];
            items.extend(resource_selector.get_texture_items());
            egui::ComboBox::from_id_salt("bumpmap")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for (_id, name, display_name) in items.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), display_name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "bsdffile" {
            let items = resource_selector.get_bsdffile_items();
            egui::ComboBox::from_id_salt("bsdffile")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for (_id, name, display_name) in items.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), display_name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name.starts_with("namedmaterial") {
            let mut items = resource_selector.get_material_items();
            if let Some(id) = own_id {
                items = items
                    .iter()
                    .filter(|(item_id, _, _)| *item_id != id)
                    .cloned()
                    .collect::<Vec<(Uuid, String, String)>>();
            }
            egui::ComboBox::from_id_salt("namedmaterial")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for (_id, name, display_name) in items.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), display_name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_type == "texture" {
            let mut items = resource_selector.get_texture_items();
            if let Some(id) = own_id {
                items = items
                    .iter()
                    .filter(|(item_id, _, _)| *item_id != id)
                    .cloned()
                    .collect::<Vec<(Uuid, String, String)>>();
            }
            egui::ComboBox::from_id_salt("texture")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for (_id, name, display_name) in items.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), display_name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else if key_name == "filename" {
            let mut s = value[0].clone();
            ui.text_edit_singleline(&mut s);
        } else if key_name == "name" {
            let names = SubsurfaceProperties::get_names();
            egui::ComboBox::from_id_salt("subsurface_names")
                .selected_text(value[0].clone())
                .show_ui(ui, |ui| {
                    for name in names.iter() {
                        if ui
                            .selectable_value(&mut value[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        } else {
            if ui.text_edit_singleline(&mut value[0]).changed() {
                is_changed = true;
            }
        }
    }
    return is_changed;
}

fn show_bools(ui: &mut egui::Ui, key_type: &str, key_name: &str, value: &mut Vec<bool>) {
    if value.len() == 1 {
        Checkbox::without_text(&mut value[0]).ui(ui);
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ColorType {
    Value,
    Texture,
    Spd,
}

fn show_color_like(
    ui: &mut egui::Ui,
    key_type: &str,
    key_name: &str,
    props: &mut PropertyMap,
    resource_selector: &ResourceSelector,
    own_id: Option<Uuid>,
) -> bool {
    let mut is_changed = false;
    //let rect = ui.available_rect_before_wrap();
    //ui.painter().rect_filled(rect, 1.0, egui::Color32::RED);
    let mut color_type = ColorType::Value;
    if key_type == "color" || key_type == "rgb" || key_type == "xyz" {
        color_type = ColorType::Value;
    } else if key_type == "spectrum" {
        color_type = ColorType::Spd;
    }
    if key_type == "texture" {
        color_type = ColorType::Texture;
    }

    let mut backups = Vec::new();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
            ui.horizontal(|ui| {
                if color_type != ColorType::Spd {
                    if ui.small_button("S").clicked() {
                        is_changed = true;
                        let search_key = format!("{}_{:?}", key_name, ColorType::Spd);
                        let backup_value = if let Some(p) = props.get(&search_key) {
                            Some(p.clone())
                        } else {
                            None
                        };

                        if let Some((key_type, _, prop)) = props.entry_mut(key_name) {
                            backups.push((
                                key_type.clone(),
                                format!("{}_{:?}", key_name, color_type),
                                prop.clone(),
                            ));
                            *key_type = "spectrum".to_string();
                            if let Some(p) = backup_value {
                                *prop = p;
                            } else {
                                *prop = Property::Strings(vec!["".to_string()]);
                            }
                        }
                    }
                }
                if color_type != ColorType::Texture {
                    if ui.small_button("T").clicked() {
                        is_changed = true;
                        let search_key = format!("{}_{:?}", key_name, ColorType::Texture);
                        let backup_value = if let Some(p) = props.get(&search_key) {
                            Some(p.clone())
                        } else {
                            None
                        };

                        if let Some((key_type, _, prop)) = props.entry_mut(key_name) {
                            backups.push((
                                key_type.clone(),
                                format!("{}_{:?}", key_name, color_type),
                                prop.clone(),
                            ));
                            *key_type = "texture".to_string();
                            if let Some(p) = backup_value {
                                *prop = p;
                            } else {
                                *prop = Property::Strings(vec!["".to_string()]);
                            }
                        }
                    }
                }
                if color_type != ColorType::Value {
                    if ui.small_button("V").clicked() {
                        is_changed = true;
                        let search_key = format!("{}_{:?}", key_name, ColorType::Value);
                        let backup_value = if let Some((t, _, p)) = props.entry(&search_key) {
                            Some((t.clone(), p.clone()))
                        } else {
                            None
                        };

                        if let Some((key_type, _, prop)) = props.entry_mut(key_name) {
                            backups.push((
                                key_type.clone(),
                                format!("{}_{:?}", key_name, color_type),
                                prop.clone(),
                            ));

                            if let Some((t, p)) = backup_value {
                                *key_type = t;
                                *prop = p;
                            } else {
                                *key_type = "color".to_string();
                                *prop = Property::Floats(vec![0.0, 0.0, 0.0]);
                            }
                        }
                    }
                }
            });
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                if key_type == "color" || key_type == "rgb" {
                    if let Some(v) = props.get_mut(key_name) {
                        if let Property::Floats(value) = v {
                            ui.horizontal(|ui| {
                                if show_rgb(ui, value) {
                                    is_changed = true;
                                }
                            });
                        }
                    }
                } else if key_type == "xyz" {
                    if let Some(v) = props.get_mut(key_name) {
                        if let Property::Floats(value) = v {
                            let mut rgb = xyz_to_rgb(&value);
                            ui.horizontal(|ui| {
                                if show_rgb(ui, &mut rgb) {
                                    is_changed = true;
                                }
                            });
                            let xyz = rgb_to_xyz(&rgb);
                            value[0] = xyz[0];
                            value[1] = xyz[1];
                            value[2] = xyz[2];
                        }
                    }
                } else if key_type == "spectrum" {
                    if let Some(v) = props.get_mut(key_name) {
                        //if let Property::Strings(value) = v {
                        //    ui.text_edit_singleline(&mut value[0]);
                        //}
                        if let Property::Strings(value) = v {
                            let spd_names = resource_selector.get_spd_items();
                            egui::ComboBox::from_id_salt("spectrum")
                                .selected_text(value[0].clone())
                                .show_ui(ui, |ui| {
                                    for (_id, name, display_name) in spd_names.iter() {
                                        if ui
                                            .selectable_value(
                                                &mut value[0],
                                                name.clone(),
                                                display_name.clone(),
                                            )
                                            .changed()
                                        {
                                            is_changed = true;
                                        }
                                    }
                                });
                        }
                    }
                } else if key_type == "texture" {
                    if let Some(v) = props.get_mut(key_name) {
                        //if let Property::Strings(value) = v {
                        //    ui.text_edit_singleline(&mut value[0]);
                        //}
                        if let Property::Strings(value) = v {
                            let mut items = resource_selector.get_texture_items();
                            if let Some(id) = own_id {
                                items = items
                                    .iter()
                                    .filter(|(item_id, _, _)| *item_id != id)
                                    .cloned()
                                    .collect::<Vec<(Uuid, String, String)>>();
                            }
                            egui::ComboBox::from_id_salt("texture")
                                .selected_text(value[0].clone())
                                .show_ui(ui, |ui| {
                                    for (_id, name, display_name) in items.iter() {
                                        if ui
                                            .selectable_value(
                                                &mut value[0],
                                                name.clone(),
                                                display_name.clone(),
                                            )
                                            .changed()
                                        {
                                            is_changed = true;
                                        }
                                    }
                                });
                        }
                    }
                }
            });
        });
    });
    for (t, n, p) in backups.iter() {
        let key = PropertyMap::get_key(t, n);
        props.insert(&key, p.clone());
    }
    return is_changed;
}

fn is_color_like(key_type: &str, key_name: &str) -> bool {
    if key_type == "color" || key_type == "rgb" || key_type == "xyz" || key_type == "spectrum" {
        return true;
    }
    if key_type == "texture" {
        if key_name != "bumpmap" {
            return true;
        }
    }
    //if key_name.starts_with("tex") {
    //    return true;
    //}
    return false;
}

pub fn show_properties(
    index: usize,
    ui: &mut egui::Ui,
    props: &mut PropertyMap,
    keys: &[(String, String, Option<ValueRange>)],
    resource_selector: &ResourceSelector,
) -> bool {
    let mut is_changed = false;
    egui_extras::TableBuilder::new(ui)
        .id_salt(index)
        .column(egui_extras::Column::initial(100.0))
        .column(egui_extras::Column::remainder())
        .auto_shrink([false, true])
        .body(|mut body| {
            let own_id = props.find_one_string("string id");
            let own_id = if let Some(id) = own_id {
                Some(Uuid::parse_str(&id).unwrap_or(Uuid::default()))
            } else {
                None
            };
            for (key_type, key_name, range) in keys.iter() {
                //let label_name = key_name.to_case(Case::Title);
                let label_name = key_name.to_string();
                body.row(18.0, |mut row| {
                    row.col(|ui| {
                        ui.label(&format!("{}", label_name));
                    });
                    row.col(|ui| {
                        let mut key_type = key_type.clone();
                        if let Some((t, _, _)) = props.entry(key_name) {
                            key_type = t.clone();
                        }
                        let key_type = &key_type;
                        if is_color_like(key_type, key_name) {
                            if show_color_like(
                                ui,
                                key_type,
                                key_name,
                                props,
                                resource_selector,
                                own_id,
                            ) {
                                is_changed = true;
                            }
                        } else {
                            if let Some(v) = props.get_mut(key_name) {
                                if let Property::Floats(value) = v {
                                    if show_floats(ui, key_type, key_name, range, value) {
                                        is_changed = true;
                                    }
                                } else if let Property::Ints(value) = v {
                                    if show_ints(ui, key_type, key_name, range, value) {
                                        is_changed = true;
                                    }
                                } else if let Property::Strings(value) = v {
                                    show_strings(
                                        ui,
                                        key_type,
                                        key_name,
                                        value,
                                        resource_selector,
                                        own_id,
                                    );
                                } else if let Property::Bools(value) = v {
                                    show_bools(ui, key_type, key_name, value);
                                }
                            } else {
                                ui.label("No property found");
                            }
                        }
                        if is_changed {
                            props.add_string("string edition", &Uuid::new_v4().to_string());
                        }
                    });
                });
            }
        });
    return is_changed;
}

pub fn show_type(ui: &mut egui::Ui, props: &mut PropertyMap, types: &[String]) -> bool {
    let mut is_changed = false;
    if let Some(v) = props.get_mut("type") {
        if let Property::Strings(s) = v {
            egui::ComboBox::from_id_salt("type")
                .selected_text(s[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        if ui
                            .selectable_value(&mut s[0], name.clone(), name.clone())
                            .changed()
                        {
                            is_changed = true;
                        }
                    }
                });
        }
    }
    return is_changed;
}

pub fn show_component_props(
    index: usize,
    title: &str,
    ui: &mut egui::Ui,
    props: &mut PropertyMap,
    keys: &[(String, String, Option<ValueRange>)],
    resource_selector: &ResourceSelector,
) -> bool {
    let mut is_changed = false;
    egui::TopBottomPanel::top(format!("{}_{}", title, index))
        .min_height(MIN_COMPONENT_HEIGHT)
        .show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(title);
            });
            ui.separator();
            if show_properties(index, ui, props, &keys, resource_selector) {
                is_changed = true;
            }
            ui.add_space(3.0);
        });
    return is_changed;
}
