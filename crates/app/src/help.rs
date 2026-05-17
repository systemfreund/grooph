use crate::Grooph;
use crate::Mode;
use crate::tools::{DeleteOp, EditOp, MetaOp, NavOp, Tool, ToolKind, all_tools};
use eframe::egui;
use eframe::egui::RichText;

impl Grooph {
    pub(super) fn help_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("info").show_animated_inside(ui, self.ui.mode == Mode::Help, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    "This app is in early development. Please report any bugs or feature requests.",
                );
                ui.hyperlink_to("Email: hello@grooph.app", "mailto:hello@grooph.app");
            });
            ui.separator();
            ui.collapsing("Keybindings", |ui| {
                ui.label(RichText::new(keybindings_text()).monospace().size(16.0));
            });

            ui.collapsing("Mouse/Finger controls", |ui| {
                let text = RichText::new(
                    "       Click/Tap: Move cursor
            Drag: Move cursor
Double-click/Tap: Toggle Note/Rest",
                )
                .monospace()
                .size(16.0);
                ui.label(text);
            });
        });
    }
}

/// Builds the keybindings block from the tool registry plus app-global shortcuts.
/// Adding a shortcut to `tools.rs` is enough — the help panel updates automatically.
fn keybindings_text() -> String {
    let mut entries: Vec<(String, String)> = Vec::new();
    // App-global shortcuts that don't live in the tool registry (handled directly
    // in `keyboard_input.rs`).
    entries.push(("Space".to_string(), "Play/pause".to_string()));
    entries.push(("Esc".to_string(), "Toggle between edit mode and playback mode".to_string()));

    for tool in all_tools() {
        for shortcut in tool.shortcuts {
            entries.push((shortcut.label(), help_label(tool).to_string()));
        }
    }

    let max_key = entries.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    entries
        .into_iter()
        .map(|(k, label)| format!("{:>width$}: {}", k, label, width = max_key))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Description of a tool for the help panel. Falls back to `Tool.label`, but
/// overrides it for tools whose palette label is a symbol that wouldn't read
/// well in keybinding help (e.g. `⟲` for Undo, `←` for Navigate Left).
fn help_label(tool: &Tool) -> &'static str {
    match tool.kind {
        ToolKind::Edit(EditOp::Undo) => "Undo",
        ToolKind::Edit(EditOp::Redo) => "Redo",
        ToolKind::Meta(MetaOp::ResetMeasure) => "Reset measure",
        ToolKind::Meta(MetaOp::ChangeTimeSignature) => "Change time signature",
        ToolKind::Delete(DeleteOp::Forward) => "Delete note (forward)",
        ToolKind::Delete(DeleteOp::Backward) => "Delete note (backward)",
        ToolKind::Navigate(NavOp::Left) => "Move cursor left",
        ToolKind::Navigate(NavOp::Right) => "Move cursor right",
        ToolKind::Navigate(NavOp::Start) => "Jump to start",
        ToolKind::Navigate(NavOp::End) => "Jump to end",
        _ => tool.label,
    }
}
