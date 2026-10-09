//! The browser's egui drawing: the tree / search list on the left, the preview on the
//! right. Rows are painted by hand inside `show_rows`, so a list of thousands costs a
//! screenful per frame.

use super::model3d::{ModelPreview, PreviewCamera, VIEW_SIZE};
use super::{Browser, kind_color};
use crate::viewer::{self, OrbitCamera};
use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, vec2};
use jc2_formats::browse::{self, Group, Preview, PreviewKind, Row, RowKind, Target};
use std::sync::Arc;

const ROW_H: f32 = 18.0;

/// A sample being played from the sound bank view.
#[derive(Component)]
pub struct BrowserSound;

/// What a click on the left list asked for (applied after the list is drawn).
enum Click {
    Select(Target),
    Folder(usize),
    Container(usize),
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    mut contexts: EguiContexts,
    mut browser: ResMut<Browser>,
    mut model: ResMut<ModelPreview>,
    mut cams: Query<(&mut OrbitCamera, &mut Transform), With<PreviewCamera>>,
    mut commands: Commands,
    mut audio: ResMut<Assets<AudioSource>>,
    sounds: Query<Entity, With<BrowserSound>>,
) -> Result {
    let ctx = contexts.ctx_mut()?.clone();
    keys(&ctx, &mut browser);

    let mut root = egui::Ui::new(
        ctx.clone(),
        "browser".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );
    egui::Panel::bottom("status").show(&mut root, |ui| status_bar(ui, &browser));
    egui::Panel::left("files")
        .resizable(true)
        .default_size(440.0)
        .size_range(280.0..=900.0)
        .show(&mut root, |ui| left_pane(ui, &mut browser));
    egui::CentralPanel::default().show(&mut root, |ui| {
        right_pane(
            ui,
            &mut browser,
            &mut model,
            &mut cams,
            &mut commands,
            &mut audio,
            &sounds,
        )
    });
    Ok(())
}

fn keys(ctx: &egui::Context, b: &mut Browser) {
    use egui::{Key, Modifiers};
    if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
        b.focus_filter = true;
    }
    if ctx.input(|i| i.key_pressed(Key::Escape)) && b.filter.is_active() {
        b.filter = Default::default();
    }
    if !ctx.egui_wants_keyboard_input() {
        let step = ctx.input_mut(|i| {
            let mut s = 0isize;
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                s += 1;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                s -= 1;
            }
            if i.consume_key(Modifiers::NONE, Key::PageDown) {
                s += 12;
            }
            if i.consume_key(Modifiers::NONE, Key::PageUp) {
                s -= 12;
            }
            s
        });
        if step != 0 {
            b.step_selection(step);
        }
    }
}

fn size_text(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / 1_048_576.0)
    } else if n >= 1 << 10 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn status_bar(ui: &mut egui::Ui, b: &Browser) {
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} files ({} named), {} archives",
            thousands(b.index.items.len()),
            thousands(b.index.named()),
            b.index.archives.len()
        ));
        ui.separator();
        if b.scan.complete() {
            ui.label("containers indexed");
        } else {
            ui.label(format!(
                "reading containers: {} / {}",
                thousands(b.scan.done),
                thousands(b.scan.total())
            ));
        }
        if !b.status.is_empty() {
            ui.separator();
            ui.label(&b.status);
        }
    });
}

// ───────────────────────────── left: the tree ─────────────────────────────

fn left_pane(ui: &mut egui::Ui, b: &mut Browser) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let edit = egui::TextEdit::singleline(&mut b.filter.text)
            .hint_text("filter by name, hash or archive (Ctrl+F)")
            .desired_width(ui.available_width() - 26.0);
        let r = ui.add(edit);
        if b.focus_filter {
            r.request_focus();
            b.focus_filter = false;
        }
        if ui.small_button("x").on_hover_text("clear (Esc)").clicked() {
            b.filter = Default::default();
        }
    });
    ui.horizontal(|ui| {
        let label = match b.filter.kind {
            Some(k) => browse::kind_label(k),
            None => "any kind",
        };
        egui::ComboBox::from_id_salt("kind")
            .selected_text(label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut b.filter.kind, None, "any kind");
                for (k, l) in browse::KINDS {
                    ui.selectable_value(&mut b.filter.kind, Some(*k), *l);
                }
            });
        let mut group = b.group;
        egui::ComboBox::from_id_salt("group")
            .selected_text(match group {
                Group::Path => "by folder",
                Group::Archive => "by archive, then folder",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut group, Group::Path, "by folder");
                ui.selectable_value(&mut group, Group::Archive, "by archive, then folder");
            });
        b.set_group(group);
    });
    match &b.results {
        Some(r) => {
            ui.label(if r.total > r.hits.len() {
                format!(
                    "{} matches, the first {} listed",
                    thousands(r.total),
                    thousands(r.hits.len())
                )
            } else {
                format!("{} matches", thousands(r.total))
            });
        }
        None => {
            ui.weak("click a folder or container to open it; Up / Down move");
        }
    }
    ui.separator();
    list(ui, b);
}

fn list(ui: &mut egui::Ui, b: &mut Browser) {
    let total = match &b.results {
        Some(r) => r.hits.len(),
        None => b.rows.len(),
    };
    let mut area = egui::ScrollArea::vertical()
        .id_salt("list")
        .auto_shrink([false, false]);
    if let Some(row) = b.scroll_to_row.take() {
        let off = (row as f32 * (ROW_H + ui.spacing().item_spacing.y) - 120.0).max(0.0);
        area = area.vertical_scroll_offset(off);
    }
    let mut click: Option<Click> = None;
    area.show_rows(ui, ROW_H, total, |ui, range| {
        for i in range {
            let c = match &b.results {
                Some(r) => hit_row(ui, b, &r.hits[i]),
                None => tree_row(ui, b, &b.rows[i]),
            };
            if click.is_none() {
                click = c;
            }
        }
    });
    match click {
        Some(Click::Select(t)) => b.select(t),
        Some(Click::Folder(f)) => b.toggle_folder(f),
        Some(Click::Container(i)) => b.toggle_container(i),
        None => {}
    }
}

fn draw_arrow(p: &egui::Painter, c: Pos2, open: bool, color: Color32) {
    let pts = if open {
        vec![
            c + vec2(-4.0, -2.5),
            c + vec2(4.0, -2.5),
            c + vec2(0.0, 3.5),
        ]
    } else {
        vec![
            c + vec2(-2.5, -4.0),
            c + vec2(-2.5, 4.0),
            c + vec2(3.5, 0.0),
        ]
    };
    p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
}

struct RowLook<'a> {
    depth: usize,
    /// `Some(open)` draws an expand arrow.
    arrow: Option<bool>,
    label: &'a str,
    detail: &'a str,
    color: Color32,
    selected: bool,
}

/// Paint one list row; returns the response and whether the click landed on the arrow.
fn paint_row(ui: &mut egui::Ui, look: RowLook) -> (egui::Response, bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return (resp, false);
    }
    let p = ui.painter_at(rect);
    if look.selected {
        p.rect_filled(rect, 2.0, ui.visuals().selection.bg_fill);
    } else if resp.hovered() {
        p.rect_filled(rect, 2.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let indent = look.depth as f32 * 14.0;
    let x = rect.left() + 4.0 + indent;
    let mid = rect.center().y;
    if let Some(open) = look.arrow {
        draw_arrow(&p, Pos2::new(x + 6.0, mid), open, ui.visuals().text_color());
    }
    let text_color = if look.selected {
        ui.visuals().selection.stroke.color
    } else {
        look.color
    };
    p.text(
        Pos2::new(x + 16.0, mid),
        Align2::LEFT_CENTER,
        look.label,
        FontId::proportional(13.5),
        text_color,
    );
    if !look.detail.is_empty() {
        p.text(
            Pos2::new(rect.right() - 6.0, mid),
            Align2::RIGHT_CENTER,
            look.detail,
            FontId::proportional(11.5),
            ui.visuals().weak_text_color(),
        );
    }
    let on_arrow = resp.clicked()
        && resp
            .interact_pointer_pos()
            .is_some_and(|pos| pos.x < x + 16.0);
    (resp, on_arrow)
}

fn tree_row(ui: &mut egui::Ui, b: &Browser, row: &Row) -> Option<Click> {
    let depth = row.depth as usize;
    match row.kind {
        RowKind::Folder { folder, open } => {
            let f = &b.tree.folders[folder];
            let color = ui.visuals().text_color();
            let (resp, _) = paint_row(
                ui,
                RowLook {
                    depth,
                    arrow: Some(open),
                    label: &f.name,
                    detail: &thousands(f.count),
                    color,
                    selected: false,
                },
            );
            resp.clicked().then_some(Click::Folder(folder))
        }
        RowKind::File { item } | RowKind::Container { item, .. } => {
            let it = &b.index.items[item];
            let container = matches!(row.kind, RowKind::Container { .. });
            let open = matches!(row.kind, RowKind::Container { open: true, .. });
            let kind = browse::item_kind(&b.index, &b.scan, item);
            let target = Target::File(it.hash);
            let (resp, on_arrow) = paint_row(
                ui,
                RowLook {
                    depth,
                    arrow: container.then_some(open),
                    label: &it.leaf,
                    detail: &size_text(it.size),
                    color: kind_color(kind),
                    selected: b.selected.as_ref() == Some(&target),
                },
            );
            if container && (on_arrow || resp.double_clicked()) {
                return Some(Click::Container(item));
            }
            resp.clicked().then_some(Click::Select(target))
        }
        RowKind::Member { item, member } => {
            let m = &b.scan.get(item)?.members[member];
            let target = Target::Member {
                container: b.index.items[item].hash,
                name: m.name.clone(),
            };
            let (resp, _) = paint_row(
                ui,
                RowLook {
                    depth,
                    arrow: None,
                    label: &m.name,
                    detail: &size_text(m.size as u64),
                    color: kind_color(browse::kind_from_name(&m.name)),
                    selected: b.selected.as_ref() == Some(&target),
                },
            );
            resp.clicked().then_some(Click::Select(target))
        }
    }
}

fn hit_row(ui: &mut egui::Ui, b: &Browser, hit: &browse::Hit) -> Option<Click> {
    let it = &b.index.items[hit.item];
    let (label, detail, kind, target) = match hit.member {
        None => (
            it.display(),
            size_text(it.size),
            browse::item_kind(&b.index, &b.scan, hit.item),
            Target::File(it.hash),
        ),
        Some(mi) => {
            let m = &b.scan.get(hit.item)?.members[mi];
            (
                format!("{}  [{}]", m.name, it.leaf),
                size_text(m.size as u64),
                browse::kind_from_name(&m.name),
                Target::Member {
                    container: it.hash,
                    name: m.name.clone(),
                },
            )
        }
    };
    let (resp, _) = paint_row(
        ui,
        RowLook {
            depth: 0,
            arrow: None,
            label: &label,
            detail: &detail,
            color: kind_color(kind),
            selected: b.selected.as_ref() == Some(&target),
        },
    );
    if resp.clicked() {
        return Some(Click::Select(target));
    }
    None
}

// ───────────────────────────── right: the preview ─────────────────────────────

#[allow(clippy::too_many_arguments)]
fn right_pane(
    ui: &mut egui::Ui,
    b: &mut Browser,
    model: &mut ModelPreview,
    cams: &mut Query<(&mut OrbitCamera, &mut Transform), With<PreviewCamera>>,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
    sounds: &Query<Entity, With<BrowserSound>>,
) {
    let Some(p) = b.preview.clone() else {
        model.wanted = None;
        ui.add_space(24.0);
        if let Some(t) = &b.loading {
            ui.heading(browse::target_label(&b.fs, t));
            ui.label("reading…");
        } else {
            ui.heading("Just Cause 2 asset browser");
            ui.label("Pick a file on the left to preview it.");
            ui.add_space(8.0);
            ui.weak(
                "Up / Down  move through the list      Ctrl+F  filter      Esc  clear the filter",
            );
            ui.weak("model view: left-drag orbit, right-drag pan, wheel zoom, F or double-click to frame");
        }
        return;
    };
    header(ui, b, &p);
    ui.separator();
    // the 3D scene follows the preview
    model.wanted = match p.view {
        PreviewKind::Model | PreviewKind::Container => p.model.clone(),
        _ => None,
    };
    if let Some(e) = &p.error {
        ui.colored_label(Color32::from_rgb(235, 110, 110), e);
    }
    match p.view {
        PreviewKind::Texture => texture_view(ui, b, &p),
        PreviewKind::Model => {
            model_view(ui, model, cams, 0.6);
            ui.separator();
            lines_view(ui, "model-lines", &p.lines);
        }
        PreviewKind::Container => {
            if p.model.is_some() {
                model_view(ui, model, cams, 0.55);
                ui.separator();
            }
            if !p.lines.is_empty() {
                egui::CollapsingHeader::new("model statistics")
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("stats")
                            .max_height(200.0)
                            .show(ui, |ui| {
                                for l in p.lines.iter().take(400) {
                                    ui.monospace(l);
                                }
                            });
                    });
            }
            members_view(ui, b, &p);
        }
        PreviewKind::Fsb => samples_view(ui, b, &p, commands, audio, sounds),
        _ => lines_view(ui, "lines", &p.lines),
    }
}

fn header(ui: &mut egui::Ui, b: &mut Browser, p: &Arc<Preview>) {
    ui.add_space(2.0);
    ui.heading(&p.label);
    let hash = match &p.target {
        Target::File(h) => *h,
        Target::Member { name, .. } => jc2_formats::hash::name_hash(name),
    };
    let archive = b
        .index
        .by_hash(p.target.file_hash())
        .map(|i| b.index.archives[b.index.items[i].archive].clone())
        .unwrap_or_default();
    ui.horizontal_wrapped(|ui| {
        ui.monospace(format!("hash {hash:08x}"));
        ui.label(format!("· {archive}"));
        ui.label(format!("· {}", browse::kind_label(p.kind)));
        ui.label(if p.compressed {
            format!(
                "· {} stored, {} inflated",
                size_text(p.stored as u64),
                size_text(p.size as u64)
            )
        } else {
            format!("· {}", size_text(p.size as u64))
        });
        ui.label(format!("· shown as {}", p.view.label()));
    });
    for l in &p.info {
        ui.weak(l);
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Copy name").clicked() {
            ui.ctx().copy_text(p.label.clone());
            info!("{}  (hash {hash:08x})", p.label);
            b.status = format!("copied {}", p.label);
        }
        if ui.button("Copy hash").clicked() {
            ui.ctx().copy_text(format!("{hash:08x}"));
            info!("{hash:08x}  ({})", p.label);
            b.status = format!("copied {hash:08x}");
        }
        if ui
            .button("Export")
            .on_hover_text("write the file as stored in the archive, as jc2tool extract does")
            .clicked()
        {
            b.export(false);
        }
        if p.compressed
            && ui
                .button("Export inflated")
                .on_hover_text("write it with the zlib wrapper removed")
                .clicked()
        {
            b.export(true);
        }
        if let Some(m) = &p.model
            && ui
                .button("Open in model viewer")
                .on_hover_text("starts `jc2 --model` on this in a new window")
                .clicked()
        {
            let m = m.clone();
            b.relaunch(&["--model", &m]);
        }
        if let Some((x, z)) = p.tile
            && ui
                .button("Open in world")
                .on_hover_text("starts `jc2 --world` on this tile in a new window")
                .clicked()
        {
            b.relaunch(&["--world", &format!("{x},{z}")]);
        }
        if !p.lines.is_empty()
            && !matches!(p.view, PreviewKind::Container)
            && ui.button("Copy text").clicked()
        {
            ui.ctx().copy_text(p.lines.join("\n"));
            b.status = format!("copied {} lines", p.lines.len());
        }
    });
    ui.horizontal(|ui| {
        ui.label("export to");
        ui.add(egui::TextEdit::singleline(&mut b.export_dir).desired_width(ui.available_width()));
    });
}

/// Monospace lines, only the visible ones laid out.
fn lines_view(ui: &mut egui::Ui, id: &str, lines: &[String]) {
    let row_h = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .id_salt(id)
        .auto_shrink([false, false])
        .show_rows(ui, row_h, lines.len(), |ui, range| {
            for i in range {
                ui.add(
                    egui::Label::new(egui::RichText::new(&lines[i]).monospace())
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });
}

fn texture_view(ui: &mut egui::Ui, b: &mut Browser, p: &Arc<Preview>) {
    let Some(img) = &p.texture else {
        lines_view(ui, "tex-lines", &p.lines);
        return;
    };
    if b.tex_opts.for_target.as_ref() != Some(&p.target) {
        // a new texture: the biggest mip that is not absurd to decode, no alpha
        b.tex_opts.for_target = Some(p.target.clone());
        b.tex_opts.mip = (0..img.mips)
            .find(|&l| {
                let (w, h) = img.level_size(l);
                w.max(h) <= 2048
            })
            .unwrap_or(img.mips - 1);
        b.tex_opts.alpha = false;
        b.tex_opts.actual = false;
    }
    ui.horizontal(|ui| {
        let (w, h) = img.level_size(b.tex_opts.mip);
        egui::ComboBox::from_id_salt("mip")
            .selected_text(format!("mip {} ({w}×{h})", b.tex_opts.mip))
            .show_ui(ui, |ui| {
                for l in 0..img.mips {
                    let (w, h) = img.level_size(l);
                    ui.selectable_value(&mut b.tex_opts.mip, l, format!("mip {l} ({w}×{h})"));
                }
            });
        ui.checkbox(&mut b.tex_opts.alpha, "alpha");
        ui.checkbox(&mut b.tex_opts.actual, "actual size");
    });
    let mip = b.tex_opts.mip.min(img.mips - 1);
    let (w, h) = img.level_size(mip);
    // room for the size line under the toolbar
    let avail = ui.available_size() - vec2(8.0, 28.0);
    let scale = if b.tex_opts.actual {
        1.0
    } else {
        (avail.x / w as f32)
            .min(avail.y / h as f32)
            .clamp(0.01, 8.0)
    };
    let nearest = scale > 1.0;
    let key = (p.target.clone(), mip, b.tex_opts.alpha, nearest);
    if b.tex.as_ref().map(|t| &t.key) != Some(&key) {
        b.tex = img.decode_rgba(mip).map(|mut rgba| {
            if !key.2 {
                for px in rgba.as_chunks_mut::<4>().0.iter_mut() {
                    px[3] = 255;
                }
            }
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            let options = if nearest {
                egui::TextureOptions::NEAREST
            } else {
                egui::TextureOptions::LINEAR
            };
            super::TextureView {
                key: key.clone(),
                handle: ui.ctx().load_texture("browser texture", image, options),
            }
        });
    }
    let Some(tex) = &b.tex else {
        ui.label("this mip level cannot be decoded");
        return;
    };
    ui.weak(format!("{w}×{h}, shown at {:.0}%", scale * 100.0));
    let size = vec2(w as f32 * scale, h as f32 * scale);
    egui::ScrollArea::both()
        .id_salt("texture")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            let painter = ui.painter_at(rect);
            if b.tex_opts.alpha {
                checkerboard(&painter, rect);
            }
            painter.image(
                tex.handle.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        });
}

fn checkerboard(p: &egui::Painter, rect: Rect) {
    let n = 12.0;
    p.rect_filled(rect, 0.0, Color32::from_gray(60));
    let (mut y, mut row) = (rect.top(), 0);
    while y < rect.bottom() {
        let (mut x, mut col) = (rect.left(), 0);
        while x < rect.right() {
            if (row + col) % 2 == 0 {
                let cell = Rect::from_min_size(Pos2::new(x, y), vec2(n, n)).intersect(rect);
                p.rect_filled(cell, 0.0, Color32::from_gray(90));
            }
            x += n;
            col += 1;
        }
        y += n;
        row += 1;
    }
}

/// The 3D viewport, `fraction` of the height that is left.
fn model_view(
    ui: &mut egui::Ui,
    model: &mut ModelPreview,
    cams: &mut Query<(&mut OrbitCamera, &mut Transform), With<PreviewCamera>>,
    fraction: f32,
) {
    ui.horizontal(|ui| {
        ui.label("LOD");
        for l in ["1", "2", "3", "4", "all"] {
            if ui.selectable_label(model.lod == l, l).clicked() {
                model.lod = l.to_string();
            }
        }
        if ui
            .button("Frame")
            .on_hover_text("F or double-click")
            .clicked()
        {
            model.reframe = true;
        }
        let (total, loaded, failed) = model.counts;
        ui.weak(format!(
            "{loaded} / {total} models{}{}{}",
            if failed > 0 {
                format!(", {failed} failed")
            } else {
                String::new()
            },
            if model.from_doll {
                ", assembled from the vehicle doll"
            } else {
                ""
            },
            if model.truncated {
                format!(" (first {})", super::model3d::MAX_MODELS)
            } else {
                String::new()
            }
        ));
        ui.weak("drag orbit · right-drag pan · wheel zoom");
    });
    let aspect = VIEW_SIZE.0 as f32 / VIEW_SIZE.1 as f32;
    let avail = ui.available_size();
    let h = (avail.y * fraction).clamp(160.0, 900.0);
    let w = (h * aspect).min(avail.x);
    let size = vec2(w, w / aspect);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    ui.painter().image(
        model.texture,
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );
    let (total, loaded, failed) = model.counts;
    if total > 0 && loaded + failed < total {
        ui.painter().text(
            rect.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            format!("loading {loaded} / {total}…"),
            FontId::proportional(13.0),
            Color32::from_gray(200),
        );
    } else if total > 0 && loaded == 0 {
        ui.painter().text(
            rect.center(),
            Align2::CENTER_CENTER,
            "no model could be loaded",
            FontId::proportional(14.0),
            Color32::from_rgb(235, 110, 110),
        );
    }
    let (drag, scroll, hovered) = (
        resp.drag_delta(),
        ui.input(|i| i.smooth_scroll_delta.y),
        resp.hovered(),
    );
    if resp.double_clicked() || (hovered && ui.input(|i| i.key_pressed(egui::Key::F))) {
        model.reframe = true;
    }
    for (mut cam, mut tf) in cams.iter_mut() {
        if resp.dragged_by(egui::PointerButton::Primary) {
            cam.yaw -= drag.x * 0.006;
            cam.pitch = (cam.pitch + drag.y * 0.006).clamp(-1.5, 1.5);
        }
        if resp.dragged_by(egui::PointerButton::Secondary) {
            let (right, up) = (tf.right(), tf.up());
            let k = cam.distance * 0.0015;
            cam.target += (-right * drag.x + up * drag.y) * k;
        }
        if hovered && scroll != 0.0 {
            cam.distance = (cam.distance * (1.0 - scroll * 0.002)).clamp(0.2, 2000.0);
        }
        viewer::apply(&cam, &mut tf);
    }
}

fn members_view(ui: &mut egui::Ui, b: &mut Browser, p: &Arc<Preview>) {
    ui.label(format!("{} members", thousands(p.members.len())));
    let container = p.target.file_hash();
    let mut chosen: Option<Target> = None;
    egui::ScrollArea::vertical()
        .id_salt("members")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, p.members.len(), |ui, range| {
            for i in range {
                let m = &p.members[i];
                let target = Target::Member {
                    container,
                    name: m.name.clone(),
                };
                let detail = format!(
                    "{}  {}",
                    m.kind.map_or("", browse::kind_label),
                    size_text(m.size as u64)
                );
                let (resp, _) = paint_row(
                    ui,
                    RowLook {
                        depth: 0,
                        arrow: None,
                        label: &m.name,
                        detail: &detail,
                        color: kind_color(m.kind),
                        selected: b.selected.as_ref() == Some(&target),
                    },
                );
                if resp.clicked() {
                    chosen = Some(target);
                }
            }
        });
    if let Some(t) = chosen {
        b.reveal(&t);
        b.select(t);
    }
}

fn samples_view(
    ui: &mut egui::Ui,
    b: &mut Browser,
    p: &Arc<Preview>,
    commands: &mut Commands,
    audio: &mut Assets<AudioSource>,
    sounds: &Query<Entity, With<BrowserSound>>,
) {
    ui.horizontal(|ui| {
        ui.label(format!("{} samples", thousands(p.samples.len())));
        if ui.button("Stop").clicked() {
            for e in sounds.iter() {
                commands.entity(e).despawn();
            }
        }
    });
    let mut play: Option<usize> = None;
    let row_h = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("samples")
        .auto_shrink([false, false])
        .show_rows(ui, row_h, p.samples.len(), |ui, range| {
            for i in range {
                let s = &p.samples[i];
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(s.playable, egui::Button::new("play"))
                        .clicked()
                    {
                        play = Some(s.index);
                    }
                    ui.monospace(format!(
                        "{:4} {:7.2}s {:5} Hz {}ch {:>9}  {}",
                        s.index,
                        s.seconds,
                        s.hz,
                        s.channels,
                        size_text(s.size as u64),
                        s.name
                    ));
                });
            }
        });
    if let Some(i) = play {
        let target = p.target.clone();
        match b
            .bank_bytes(&target)
            .and_then(|bytes| browse::fsb_sample_mp3(&bytes, i))
        {
            Some(mp3) => {
                for e in sounds.iter() {
                    commands.entity(e).despawn();
                }
                commands.spawn((
                    AudioPlayer(audio.add(AudioSource { bytes: mp3.into() })),
                    PlaybackSettings::DESPAWN,
                    BrowserSound,
                ));
                b.status = format!("playing {}", p.samples[i].name);
            }
            None => b.status = "that sample cannot be played".into(),
        }
    }
}
