//! Abstract tool model and static registry for the tool palette.
//!
//! This module defines data-only types (no UI/logic) so that the palette
//! can be rendered dynamically without hardcoding entries in `app.rs`.

use eframe::egui::{InputState, Key};
use grooph_measure::BeatKind;
use grooph_measure::duration::NoteValue::{Eighth, Quarter, Sixteenth, ThirtySecond};
use grooph_measure::duration::{Duration, TupletSpec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolGroup {
    Notes,
    Rests,
    Tuplets,
    Modifiers,
    Edit,
    Meta,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeatTemplate {
    pub duration: Duration,
    pub kind: BeatKind,
    pub accented: bool
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    ToggleDotted { dots: u8 },
    ToggleAccent,
    ToggleRestNote,
    CycleTuplet,
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
pub enum ToolKind {
    InsertBeat(BeatTemplate),
    Modify(Modifier),
    Edit(EditOp),
    Meta(MetaOp),
}

impl ToolKind {
    /// Whether this tool mutates the measure and therefore needs an undo snapshot.
    /// Undo/Redo manage their own stacks; opening the time-signature dialog is not a mutation.
    pub fn is_mutating(&self) -> bool {
        match self {
            ToolKind::InsertBeat(..) => true,
            ToolKind::Modify(..) => true,
            ToolKind::Meta(MetaOp::ResetMeasure) => true,
            ToolKind::Edit(_) => false,
            ToolKind::Meta(MetaOp::ChangeTimeSignature) => false,
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
    pub const fn plain(key: Key) -> Self { Self { key, shift: false, command: false } }

    pub fn is_pressed(&self, i: &InputState) -> bool {
        if !i.key_pressed(self.key) {
            return false;
        }
        let command_pressed = i.modifiers.command || i.modifiers.ctrl;
        i.modifiers.shift == self.shift && command_pressed == self.command
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: &'static str,
    pub label: &'static str,
    pub group: ToolGroup,
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

    static ALL: [Tool; 22] = [
        Tool {
            id: "edit.undo",
            label: "⟲",
            group: ToolGroup::Edit,
            kind: ToolKind::Edit(EditOp::Undo),
            shortcuts: &[Shortcut { key: Key::Z, shift: false, command: true }],
            show_in_palette: true,
        },
        Tool {
            id: "edit.redo",
            label: "⟳",
            group: ToolGroup::Edit,
            kind: ToolKind::Edit(EditOp::Redo),
            shortcuts: &[
                Shortcut { key: Key::Z, shift: true, command: true },
                Shortcut { key: Key::Y, shift: false, command: true },
            ],
            show_in_palette: true,
        },
        // Notes
        Tool {
            id: "insert.note.q",
            label: "Quarter",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Quarter), kind: Note, accented: false }),
            shortcuts: &[Shortcut::plain(Key::Num1)],
            show_in_palette: true,
        },
        Tool {
            id: "insert.note.e",
            label: "Eighth",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Eighth), kind: Note, accented: false }),
            shortcuts: &[Shortcut::plain(Key::Num2)],
            show_in_palette: true,
        },
        Tool {
            id: "insert.note.s",
            label: "Sixteenth",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Sixteenth), kind: Note, accented: false }),
            shortcuts: &[Shortcut::plain(Key::Num3)],
            show_in_palette: true,
        },
        Tool {
            id: "insert.note.th",
            label: "Thirty-Second",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(ThirtySecond), kind: Note, accented: false }),
            shortcuts: &[Shortcut::plain(Key::Num4)],
            show_in_palette: true,
        },
        // Rests
        Tool {
            id: "insert.rest.q",
            label: "Quarter rest",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Quarter), kind: Rest, accented: false }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.rest.e",
            label: "Eighth rest",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Eighth), kind: Rest, accented: false }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.rest.s",
            label: "Sixteenth rest",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Sixteenth), kind: Rest, accented: false }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.rest.th",
            label: "Thirty-Second rest",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(ThirtySecond), kind: Rest, accented: false }),
            shortcuts: &[],
            show_in_palette: true,
        },
        // Modifiers
        Tool {
            id: "modify.toggle.dotted",
            label: "Toggle Dotted",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleDotted { dots: 1 }),
            shortcuts: &[Shortcut::plain(Key::Period)],
            show_in_palette: true,
        },
        Tool {
            id: "modify.toggle.accent",
            label: "Toggle Accent",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleAccent),
            shortcuts: &[Shortcut::plain(Key::A)],
            show_in_palette: true,
        },
        Tool {
            id: "modify.toggle.rest_note",
            label: "Toggle Note/Rest",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleRestNote),
            shortcuts: &[Shortcut::plain(Key::Enter)],
            show_in_palette: false,
        },
        Tool {
            id: "modify.cycle_tuplet",
            label: "Cycle Tuplet",
            group: ToolGroup::Tuplets,
            kind: ToolKind::Modify(Modifier::CycleTuplet),
            shortcuts: &[Shortcut::plain(Key::T)],
            show_in_palette: false,
        },
        Tool {
            id: "insert.tuplet.t8",
            label: "Triplet (1/8)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 3, m: 2, base: Eighth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.tuplet.t16",
            label: "Triplet (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 3, m: 2, base: Sixteenth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.tuplet.qt16",
            label: "Quintuplet (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 5, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.tuplet.st16",
            label: "Sextuplet (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 6, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.tuplet.spt16",
            label: "Septuplet (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 7, m: 4, base: Sixteenth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "insert.tuplet.nt16",
            label: "Nonuplet (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 9, m: 8, base: Sixteenth }),
                kind: Note,
                accented: false,
            }),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "meta.reset_measure",
            label: "🗑",
            group: ToolGroup::Meta,
            kind: ToolKind::Meta(MetaOp::ResetMeasure),
            shortcuts: &[],
            show_in_palette: true,
        },
        Tool {
            id: "meta.change_time_signature",
            label: "4/4",
            group: ToolGroup::Meta,
            kind: ToolKind::Meta(MetaOp::ChangeTimeSignature),
            shortcuts: &[],
            show_in_palette: true,
        },
    ];

    &ALL
}
