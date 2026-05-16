use crate::Grooph;
use crate::Mode;
use crate::tools::all_tools;
use eframe::egui;
use eframe::egui::Key;

impl Grooph {
    pub(super) fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Modal dialogs swallow global shortcuts; their own UI provides Cancel/Confirm.
            if matches!(self.mode, Mode::TimeSignature { .. }) {
                return;
            }
            // Toggle edit mode with Escape
            if i.key_pressed(Key::Escape) {
                self.toggle_mode(Mode::Edit);
            }

            // Toggle Play/Stop with Space
            if i.key_pressed(Key::Space) {
                self.toggle_playback();
            }

            // The hard-coded edits below (cursor navigation, Delete/Backspace) are not yet modelled
            // as Tools, so they need their own mode gate. Tool shortcuts are gated centrally by
            // apply_tool's tool_applicable check.
            if self.mode != Mode::Edit {
                return;
            }

            // Undo / Redo shortcuts: Ctrl/Cmd+Z (undo), Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y (redo).
            // The Shortcut model does not yet carry Ctrl/Cmd, so these stay hard-coded for now.
            let mut consumed_undo_redo = false;
            let undo_combo = i.key_pressed(Key::Z) && (i.modifiers.command || i.modifiers.ctrl);
            let redo_combo_z = i.key_pressed(Key::Z)
                && (i.modifiers.command || i.modifiers.ctrl)
                && i.modifiers.shift;
            let redo_combo_y = i.key_pressed(Key::Y) && (i.modifiers.command || i.modifiers.ctrl);
            if undo_combo && !i.modifiers.shift {
                self.undo();
                consumed_undo_redo = true;
            } else if redo_combo_z || redo_combo_y {
                self.redo();
                consumed_undo_redo = true;
            }

            let beats_len = self.measure.beats().len();
            if beats_len > 0 && !consumed_undo_redo {
                // Navigation over committed beats only
                let mut pos = self.cursor_idx;
                if i.key_pressed(Key::ArrowLeft) {
                    pos = pos.saturating_sub(1);
                }
                if i.key_pressed(Key::ArrowRight) {
                    let max_idx = beats_len.saturating_sub(1);
                    if pos < max_idx {
                        pos += 1;
                    }
                }
                if i.key_pressed(Key::Home) {
                    pos = 0;
                }
                if i.key_pressed(Key::End) {
                    pos = beats_len.saturating_sub(1);
                }
                self.cursor_idx = pos;

                if i.key_pressed(Key::Delete) {
                    self.remove_at_cursor(CursorAdvance::Right);
                }
                if i.key_pressed(Key::Backspace) {
                    self.remove_at_cursor(CursorAdvance::Left);
                }

                // Keyboard input routed through tool shortcuts
                for t in all_tools().iter().filter(|t| t.shortcut.is_some()) {
                    let sc = t.shortcut.unwrap();
                    if i.key_pressed(sc.key) && i.modifiers.shift == sc.with_shift {
                        self.apply_tool(t);
                    }
                }
            }
        });
    }

    fn remove_at_cursor(&mut self, advance: CursorAdvance) {
        let beats_len = self.measure.beats().len();
        if beats_len == 0 {
            return;
        }
        let idx = self.cursor_idx.min(beats_len - 1);
        self.with_undo_snapshot(|g| {
            g.measure.remove(idx);
            let new_len = g.measure.beats().len();
            if new_len == 0 {
                g.cursor_idx = 0;
            } else {
                let new_pos = match advance {
                    CursorAdvance::Right => (new_len - 1).min(g.cursor_idx + 1),
                    CursorAdvance::Left => g.cursor_idx.saturating_sub(1).min(new_len - 1),
                };
                g.cursor_idx = new_pos;
            }
            true
        });
    }
}

enum CursorAdvance {
    Left,
    Right,
}
