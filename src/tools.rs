//! Abstract tool model and static registry for the tool palette.
//!
//! This module defines data-only types (no UI/logic) so that the palette
//! can be rendered dynamically without hardcoding entries in `app.rs`.

use crate::measure::BeatKind;
use crate::measure::duration::{Duration, TupletSpec};
use crate::measure::duration::NoteValue::{Eighth, Quarter, Sixteenth, ThirtySecond};

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    ToggleDotted { dots: u8 },
    ToggleAccent,
    ToggleRestNote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    Erase,
    ReplaceOnApply, // replace target with provided template (used when combined with Insert tools)
    FillToBoundary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetaOp {
    ChangeTimeSignature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    InsertBeat(BeatTemplate),
    Modify(Modifier),
    Edit(EditOp),
    Meta(MetaOp),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key: char,
    pub with_shift: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: &'static str,
    pub label: &'static str,
    pub group: ToolGroup,
    pub kind: ToolKind,
    pub shortcut: Option<Shortcut>,
}

/// Static registry of all tools shown in the palette.
pub fn all_tools() -> &'static [Tool] {
    use BeatKind::*;
    use Duration::*;

    static ALL: [Tool; 20] = [
        // Notes
        Tool {
            id: "insert.note.q",
            label: "Viertel",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Quarter), kind: Note }),
            shortcut: Some(Shortcut { key: '1', with_shift: false }),
        },
        Tool {
            id: "insert.note.e",
            label: "Achtel",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Eighth), kind: Note }),
            shortcut: Some(Shortcut { key: '2', with_shift: false }),
        },
        Tool {
            id: "insert.note.s",
            label: "Sechzehntel",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Sixteenth), kind: Note }),
            shortcut: Some(Shortcut { key: '3', with_shift: false }),
        },
        Tool {
            id: "insert.note.th",
            label: "Zweiunddreißigstel",
            group: ToolGroup::Notes,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(ThirtySecond), kind: Note }),
            shortcut: Some(Shortcut { key: '4', with_shift: false }),
        },
        // Rests
        Tool {
            id: "insert.rest.q",
            label: "Viertelpause",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Quarter), kind: Rest }),
            shortcut: None,
        },
        Tool {
            id: "insert.rest.e",
            label: "Achtelpause",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Eighth), kind: Rest }),
            shortcut: None,
        },
        Tool {
            id: "insert.rest.s",
            label: "Sechzehntelpause",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(Sixteenth), kind: Rest }),
            shortcut: None,
        },
        Tool {
            id: "insert.rest.th",
            label: "Zweiunddreißigstelpause",
            group: ToolGroup::Rests,
            kind: ToolKind::InsertBeat(BeatTemplate { duration: Simple(ThirtySecond), kind: Rest }),
            shortcut: None,
        },
        // Modifiers
        Tool {
            id: "modify.toggle.dotted",
            label: "Punktiert umschalten",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleDotted { dots: 1 }),
            shortcut: Some(Shortcut { key: '.', with_shift: false }),
        },
        Tool {
            id: "modify.toggle.accent",
            label: "Akzent umschalten",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleAccent),
            shortcut: Some(Shortcut { key: 'a', with_shift: false }),
        },
        Tool {
            id: "modify.toggle.rest_note",
            label: "Note/Rest umschalten",
            group: ToolGroup::Modifiers,
            kind: ToolKind::Modify(Modifier::ToggleRestNote),
            shortcut: Some(Shortcut { key: ' ', with_shift: false }),
        },
        // Tuplets (explicit entries, no keyboard cycle)
        Tool {
            id: "insert.tuplet.t8",
            label: "Triole (1/8)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 3, m: 2, base: Eighth }),
                kind: Note,
            }),
            shortcut: None,
        },
        Tool {
            id: "insert.tuplet.qt16",
            label: "Quintole (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 5, m: 4, base: Sixteenth }),
                kind: Note,
            }),
            shortcut: None,
        },
        Tool {
            id: "insert.tuplet.st16",
            label: "Sextole (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 6, m: 4, base: Sixteenth }),
                kind: Note,
            }),
            shortcut: None,
        },
        Tool {
            id: "insert.tuplet.spt16",
            label: "Septole (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 7, m: 4, base: Sixteenth }),
                kind: Note,
            }),
            shortcut: None,
        },
        Tool {
            id: "insert.tuplet.nt16",
            label: "Nonole (1/16)",
            group: ToolGroup::Tuplets,
            kind: ToolKind::InsertBeat(BeatTemplate {
                duration: Tuplet(TupletSpec { n: 9, m: 8, base: Sixteenth }),
                kind: Note,
            }),
            shortcut: None,
        },
        // Edit
        Tool {
            id: "edit.erase",
            label: "Löschen",
            group: ToolGroup::Edit,
            kind: ToolKind::Edit(EditOp::Erase),
            shortcut: None,
        },
        Tool {
            id: "edit.replace",
            label: "Ersetzen",
            group: ToolGroup::Edit,
            kind: ToolKind::Edit(EditOp::ReplaceOnApply),
            shortcut: None,
        },
        Tool {
            id: "edit.fill_to_boundary",
            label: "Füllen bis Grenze",
            group: ToolGroup::Edit,
            kind: ToolKind::Edit(EditOp::FillToBoundary),
            shortcut: None,
        },
        // Meta (future; placeholder visible only when used)
        Tool {
            id: "meta.change_time_signature",
            label: "Taktmaß ändern",
            group: ToolGroup::Meta,
            kind: ToolKind::Meta(MetaOp::ChangeTimeSignature),
            shortcut: None,
        },
    ];

    &ALL
}
