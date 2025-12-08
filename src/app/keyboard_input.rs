use eframe::egui;
use eframe::egui::Key;
use crate::app::PlayerState;
use crate::{all_tools, Grooph};

impl Grooph {
    pub(super) fn handle_keyboard_input(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // While the time signature dialog is open, ignore global keyboard shortcuts
            if self.show_ts_dialog {
                return;
            }
            // Toggle edit mode with Escape
            if i.key_pressed(Key::Escape) {
                self.edit_mode_enabled = !self.edit_mode_enabled;
            }

            // Toggle Play/Pause with Enter (Return)
            if i.key_pressed(Key::Enter) {
                let was_playing = self.player_state == PlayerState::Playing;
                self.player_state =
                    if was_playing { PlayerState::Paused } else { PlayerState::Playing };
                if self.player_state == PlayerState::Playing && self.audio.is_none() {
                    self.audio = crate::audio::Audio::new(self.bpm);
                }
            }

            if !self.edit_mode_enabled {
                return;
            }

            // Undo / Redo shortcuts: Ctrl/Cmd+Z (undo), Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y (redo)
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
            let total_len = beats_len;
            if total_len > 0 && !consumed_undo_redo {
                // Navigation over committed beats only
                let mut pos = self.cursor_idx;
                if i.key_pressed(Key::ArrowLeft) {
                    pos = pos.saturating_sub(1);
                }
                if i.key_pressed(Key::ArrowRight) {
                    let max_idx = total_len.saturating_sub(1);
                    if pos < max_idx {
                        pos += 1;
                    }
                }
                if i.key_pressed(Key::Home) {
                    pos = 0;
                }
                if i.key_pressed(Key::End) {
                    pos = total_len.saturating_sub(1);
                }
                self.cursor_idx = pos;

                // Edits apply only when the cursor is on a committed beat
                let idx = self.cursor_idx.min(beats_len.saturating_sub(1));
                if self.edit_mode_enabled {
                    if i.key_pressed(Key::Delete) {
                        // Remove beat at the cursor
                        self.push_undo();
                        self.measure.remove(idx);
                        // Move cursor right
                        let new_pos = (self.measure.beats().len() - 1).min(self.cursor_idx + 1);
                        self.cursor_idx = new_pos;
                        self.clear_redo();
                    }
                    if i.key_pressed(Key::Backspace) {
                        // Remove beat at the cursor
                        self.push_undo();
                        self.measure.remove(idx);
                        // Move cursor left
                        let new_len = self.measure.beats().len();
                        let new_pos = self.cursor_idx.saturating_sub(1).min(new_len - 1);
                        self.cursor_idx = new_pos;
                        self.clear_redo();
                    }
                    // Keyboard input routed through tool shortcuts
                    for t in all_tools().iter().filter(|t| t.shortcut.is_some()) {
                        let sc = t.shortcut.unwrap();
                        if let Some(key) = Self::char_to_key(sc.key) {
                            // Match exact shift requirement
                            if i.key_pressed(key) && i.modifiers.shift == sc.with_shift {
                                self.apply_tool(t);
                            }
                        }
                    }
                    if i.key_pressed(Key::T) {
                        // Snapshot before attempting tuplet cycle via hotkey
                        self.push_undo();
                        let res = self.set_tuplet(idx, None);
                        if res.is_some() {
                            self.clear_redo();
                        } else {
                            let _ = self.undo_stack.pop();
                        }
                    }
                }
            }
        });
    }

    fn char_to_key(c: char) -> Option<Key> {
        use Key::*;
        Some(match c {
            '1' => Num1,
            '2' => Num2,
            '3' => Num3,
            '4' => Num4,
            '5' => Num5,
            '6' => Num6,
            '7' => Num7,
            '8' => Num8,
            '9' => Num9,
            '0' => Num0,
            '.' => Period,
            ' ' => Space,
            'a' | 'A' => A,
            't' | 'T' => T,
            _ => return None,
        })
    }
}