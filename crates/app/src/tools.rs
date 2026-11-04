//! Abstract tool model and static registry for the tool palette.
//!
//! This module defines data-only types (no UI/logic) so that the palette
//! can be rendered dynamically without hardcoding entries in `app.rs`.

use eframe::egui::{InputState, Key};
use grooph_measure::duration::NoteValue::{Eighth, Half, Quarter, Sixteenth, ThirtySecond, Whole};
use grooph_measure::duration::{Duration, TupletSpec};
use grooph_measure::{BeatKind, Technique};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeatTemplate {
    pub duration: Duration,
    pub kind: BeatKind,
    pub accented: bool,
    pub soft: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    ToggleDotted {
        dots: u8,
    },
    ToggleAccent,
    ToggleSoft,
    ToggleRestNote,
    /// Add the chosen instrument's head to the beat, or take it away: on a
    /// staff of several instruments.
    ToggleHead,
    /// Play the chosen instrument's head with its next technique.
    CycleTechnique,
    /// Play the chosen instrument's head with this technique, or the usual
    /// way again if it already is.
    ToggleTechnique(Technique),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaOp {
    ChangeTimeSignature,
    ResetMeasure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteOp {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavOp {
    Left,
    Right,
    Start,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    InsertBeat(BeatTemplate),
    Modify(Modifier),
    Edit(EditOp),
    Meta(MetaOp),
    Delete(DeleteOp),
    Navigate(NavOp),
}

impl ToolKind {
    /// Whether this tool mutates the measure and therefore needs an undo snapshot.
    /// Undo/Redo manage their own stacks; opening the time-signature dialog is not a mutation.
    /// Navigation only moves the cursor.
    pub fn is_mutating(&self) -> bool {
        match self {
            ToolKind::InsertBeat(..) => true,
            ToolKind::Modify(..) => true,
            ToolKind::Meta(MetaOp::ResetMeasure) => true,
            ToolKind::Delete(_) => true,
            ToolKind::Edit(_) => false,
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => false,
            ToolKind::Navigate(_) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key: Key,
    pub shift: bool,
    /// Ctrl on Windows/Linux, Cmd on macOS — egui's `modifiers.command` plus raw `ctrl`.
    pub command: bool,
}

impl Shortcut {
    pub const fn plain(key: Key) -> Self {
        Self { key, shift: false, command: false }
    }

    pub fn is_pressed(&self, i: &InputState) -> bool {
        if !i.key_pressed(self.key) {
            return false;
        }
        let command_pressed = i.modifiers.command || i.modifiers.ctrl;
        i.modifiers.shift == self.shift && command_pressed == self.command
    }

    /// Friendly description suitable for help text, e.g. `Ctrl+Shift+Z`, `1`, `←`.
    /// `Cmd` is the platform-agnostic name used in egui for the modifier key
    /// that maps to Ctrl on Windows/Linux and Cmd on macOS.
    pub fn label(&self) -> String {
        let key = key_label(self.key);
        match (self.command, self.shift) {
            (true, true) => format!("Cmd+Shift+{key}"),
            (true, false) => format!("Cmd+{key}"),
            (false, true) => format!("Shift+{key}"),
            (false, false) => key,
        }
    }
}

fn key_label(k: Key) -> String {
    match k {
        Key::ArrowLeft => "←".into(),
        Key::ArrowRight => "→".into(),
        Key::ArrowUp => "↑".into(),
        Key::ArrowDown => "↓".into(),
        Key::Backspace => "Backspace".into(),
        Key::Delete => "Del".into(),
        Key::Enter => "Enter".into(),
        Key::Escape => "Esc".into(),
        Key::Space => "Space".into(),
        Key::Period => ".".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::Num1 => "1".into(),
        Key::Num2 => "2".into(),
        Key::Num3 => "3".into(),
        Key::Num4 => "4".into(),
        _ => format!("{k:?}"),
    }
}

/// Names a tool of [`all_tools`], in its order there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolId {
    EditUndo,
    EditRedo,
    InsertNoteW,
    InsertNoteH,
    InsertNoteQ,
    InsertNoteE,
    InsertNoteS,
    InsertNoteTh,
    InsertRestW,
    InsertRestH,
    InsertRestQ,
    InsertRestE,
    InsertRestS,
    InsertRestTh,
    ModifyToggleDotted,
    ModifyToggleAccent,
    ModifyToggleSoft,
    ModifyToggleRestNote,
    ModifyToggleHead,
    ModifyCycleTechnique,
    ModifyTechniqueOpen,
    ModifyTechniqueHalfOpen,
    ModifyTechniqueClosed,
    ModifyTechniqueCrossStick,
    ModifyTechniqueRimshot,
    ModifyTechniqueBell,
    ModifyTechniqueChoke,
    InsertTupletT8,
    InsertTupletT16,
    InsertTupletQt16,
    InsertTupletSt16,
    InsertTupletSpt16,
    InsertTupletNt16,
    MetaResetMeasure,
    MetaChangeTimeSignature,
    EditDeleteForward,
    EditDeleteBackward,
    NavLeft,
    NavRight,
    NavStart,
    NavEnd,
}

impl ToolId {
    /// The tool's stable name, e.g. `insert.note.q` (in logs).
    pub const fn name(self) -> &'static str {
        match self {
            ToolId::EditUndo => "edit.undo",
            ToolId::EditRedo => "edit.redo",
            ToolId::InsertNoteW => "insert.note.w",
            ToolId::InsertNoteH => "insert.note.h",
            ToolId::InsertNoteQ => "insert.note.q",
            ToolId::InsertNoteE => "insert.note.e",
            ToolId::InsertNoteS => "insert.note.s",
            ToolId::InsertNoteTh => "insert.note.th",
            ToolId::InsertRestW => "insert.rest.w",
            ToolId::InsertRestH => "insert.rest.h",
            ToolId::InsertRestQ => "insert.rest.q",
            ToolId::InsertRestE => "insert.rest.e",
            ToolId::InsertRestS => "insert.rest.s",
            ToolId::InsertRestTh => "insert.rest.th",
            ToolId::ModifyToggleDotted => "modify.toggle.dotted",
            ToolId::ModifyToggleAccent => "modify.toggle.accent",
            ToolId::ModifyToggleSoft => "modify.toggle.soft",
            ToolId::ModifyToggleRestNote => "modify.toggle.rest_note",
            ToolId::ModifyToggleHead => "modify.toggle.head",
            ToolId::ModifyCycleTechnique => "modify.cycle_technique",
            ToolId::ModifyTechniqueOpen => "modify.technique.open",
            ToolId::ModifyTechniqueHalfOpen => "modify.technique.half_open",
            ToolId::ModifyTechniqueClosed => "modify.technique.closed",
            ToolId::ModifyTechniqueCrossStick => "modify.technique.cross_stick",
            ToolId::ModifyTechniqueRimshot => "modify.technique.rimshot",
            ToolId::ModifyTechniqueBell => "modify.technique.bell",
            ToolId::ModifyTechniqueChoke => "modify.technique.choke",
            ToolId::InsertTupletT8 => "insert.tuplet.t8",
            ToolId::InsertTupletT16 => "insert.tuplet.t16",
            ToolId::InsertTupletQt16 => "insert.tuplet.qt16",
            ToolId::InsertTupletSt16 => "insert.tuplet.st16",
            ToolId::InsertTupletSpt16 => "insert.tuplet.spt16",
            ToolId::InsertTupletNt16 => "insert.tuplet.nt16",
            ToolId::MetaResetMeasure => "meta.reset_measure",
            ToolId::MetaChangeTimeSignature => "meta.change_time_signature",
            ToolId::EditDeleteForward => "edit.delete.forward",
            ToolId::EditDeleteBackward => "edit.delete.backward",
            ToolId::NavLeft => "nav.left",
            ToolId::NavRight => "nav.right",
            ToolId::NavStart => "nav.start",
            ToolId::NavEnd => "nav.end",
        }
    }

    /// How many tools there are: `all_tools` has one entry per variant, so its
    /// array is this long and an id without an entry fails to compile. Keep
    /// [`ToolId::NavEnd`] last, or name the new last variant here.
    pub const COUNT: usize = ToolId::NavEnd as usize + 1;

    /// The tool it names.
    pub fn tool(self) -> &'static Tool {
        &all_tools()[self as usize]
    }
}

impl std::fmt::Display for ToolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: ToolId,
    pub label: &'static str,
    pub kind: ToolKind,
    pub shortcuts: &'static [Shortcut],
    pub show_in_palette: bool,
}

impl Tool {
    pub fn matches(&self, i: &InputState) -> bool {
        self.shortcuts.iter().any(|sc| sc.is_pressed(i))
    }
}

/// Returns the first tool whose shortcut matches the current input state.
pub fn matching_tool(i: &InputState) -> Option<&'static Tool> {
    all_tools().iter().find(|t| t.matches(i))
}

/// Static registry of all tools shown in the palette.
pub fn all_tools() -> &'static [Tool] {
    use BeatKind::*;
    use Duration::*;

    static ALL: [Tool; ToolId::COUNT] = [
        Tool {
            id: ToolId::EditUndo,
            label: "⟲",
            kind: ToolKind::Edit(EditOp::Undo),
            shortcuts: &[Shortcut { key: Key::Z, shift: false, command: true }],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::EditRedo,
            label: "⟳",
            kind: ToolKind::Edit(EditOp::Redo),
            shortcuts: &[
                Shortcut { key: Key::Z, shift: true, command: true },
                Shortcut { key: Key::Y, shift: false, command: true },
            ],
            show_in_palette: true,
        },
        // Notes
        Tool {
            id: ToolId::InsertNoteW,
            label: "Whole",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Whole),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertNoteH,
            label: "Half",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Half),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertNoteQ,
            label: "Quarter",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Quarter),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[Shortcut::plain(Key::Num1)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertNoteE,
            label: "Eighth",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Eighth),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[Shortcut::plain(Key::Num2)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertNoteS,
            label: "Sixteenth",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Sixteenth),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[Shortcut::plain(Key::Num3)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertNoteTh,
            label: "Thirty-Second",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(ThirtySecond),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[Shortcut::plain(Key::Num4)],
            show_in_palette: true,
        },
        // Rests
        Tool {
            id: ToolId::InsertRestW,
            label: "Whole rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Whole),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertRestH,
            label: "Half rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Half),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertRestQ,
            label: "Quarter rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Quarter),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertRestE,
            label: "Eighth rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Eighth),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertRestS,
            label: "Sixteenth rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(Sixteenth),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertRestTh,
            label: "Thirty-Second rest",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Simple(ThirtySecond),
                kind: Rest,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        // Modifiers
        Tool {
            id: ToolId::ModifyToggleDotted,
            label: "Toggle Dotted",
            kind: ToolKind::Modify(Modifier::ToggleDotted { dots: 1 }),
            shortcuts: &[Shortcut::plain(Key::Period)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::ModifyToggleAccent,
            label: "Toggle Accent",
            kind: ToolKind::Modify(Modifier::ToggleAccent),
            shortcuts: &[Shortcut::plain(Key::A)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::ModifyToggleSoft,
            label: "Toggle Soft",
            kind: ToolKind::Modify(Modifier::ToggleSoft),
            shortcuts: &[Shortcut::plain(Key::G)],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::ModifyToggleRestNote,
            label: "Toggle Note/Rest",
            kind: ToolKind::Modify(Modifier::ToggleRestNote),
            shortcuts: &[Shortcut::plain(Key::Enter)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyToggleHead,
            label: "Toggle Instrument",
            kind: ToolKind::Modify(Modifier::ToggleHead),
            shortcuts: &[Shortcut::plain(Key::H)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyCycleTechnique,
            label: "Cycle Technique",
            kind: ToolKind::Modify(Modifier::CycleTechnique),
            shortcuts: &[Shortcut::plain(Key::O)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueOpen,
            label: "o",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::Open)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueHalfOpen,
            label: "ø",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::HalfOpen)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueClosed,
            label: "+",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::Closed)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueCrossStick,
            label: "×st",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::CrossStick)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueRimshot,
            label: "rim",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::Rimshot)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueBell,
            label: "bell",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::Bell)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::ModifyTechniqueChoke,
            label: "ch",
            kind: ToolKind::Modify(Modifier::ToggleTechnique(Technique::Choke)),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::InsertTupletT8,
            label: "Triplet (1/8)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 3, m: 2, base: Eighth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertTupletT16,
            label: "Triplet (1/16)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 3, m: 2, base: Sixteenth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertTupletQt16,
            label: "Quintuplet (1/16)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 5, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertTupletSt16,
            label: "Sextuplet (1/16)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 6, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertTupletSpt16,
            label: "Septuplet (1/16)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 7, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::InsertTupletNt16,
            label: "Nonuplet (1/16)",
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 9, m: 8, base: Sixteenth }),
                kind: Note,
                accented: false,
                soft: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: ToolId::MetaResetMeasure,
            label: "🗑",
            kind: ToolKind::Meta(MetaOp::ResetMeasure),
            shortcuts: &[],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::MetaChangeTimeSignature,
            label: "4/4",
            kind: ToolKind::Meta(MetaOp::ChangeTimeSignature),
            shortcuts: &[],
            show_in_palette: true,
        },
        // Beat removal — keyboard-only
        Tool {
            id: ToolId::EditDeleteForward,
            label: "Delete",
            kind: ToolKind::Delete(DeleteOp::Forward),
            shortcuts: &[Shortcut::plain(Key::Delete)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::EditDeleteBackward,
            label: "Backspace",
            kind: ToolKind::Delete(DeleteOp::Backward),
            shortcuts: &[Shortcut::plain(Key::Backspace)],
            show_in_palette: false,
        },
        // Cursor navigation — keyboard-only
        Tool {
            id: ToolId::NavLeft,
            label: "←",
            kind: ToolKind::Navigate(NavOp::Left),
            shortcuts: &[Shortcut::plain(Key::ArrowLeft)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::NavRight,
            label: "→",
            kind: ToolKind::Navigate(NavOp::Right),
            shortcuts: &[Shortcut::plain(Key::ArrowRight)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::NavStart,
            label: "Home",
            kind: ToolKind::Navigate(NavOp::Start),
            shortcuts: &[Shortcut::plain(Key::Home)],
            show_in_palette: false,
        },
        Tool {
            id: ToolId::NavEnd,
            label: "End",
            kind: ToolKind::Navigate(NavOp::End),
            shortcuts: &[Shortcut::plain(Key::End)],
            show_in_palette: false,
        },
    ];

    &ALL
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`ToolId::tool`] looks a tool up by its place in [`all_tools`].
    #[test]
    fn every_id_names_its_own_tool() {
        for (i, tool) in all_tools().iter().enumerate() {
            assert_eq!(tool.id as usize, i, "{}", tool.id);
            assert_eq!(tool.id.tool(), tool);
        }
        let mut names: Vec<_> = all_tools().iter().map(|t| t.id.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all_tools().len());
    }
}
