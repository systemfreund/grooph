use crate::duration::TupletKind;
use crate::grid::DEFAULT_GRID;
use crate::grouping::default_groups_for;
use crate::{Measure, TimeSignature};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorId(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    Scope,
    Sub,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorPattern {
    pub palette: Vec<ColorId>,
    pub mode: ColorMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelToken {
    BeatNum,
    GroupNum,
    SubNum,
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelPattern {
    pub slots: Vec<Vec<LabelToken>>,
}

impl LabelPattern {
    pub fn single(token: LabelToken) -> Self { Self { slots: vec![vec![token]] } }

    pub fn ands() -> Self {
        Self { slots: vec![vec![LabelToken::BeatNum], vec![LabelToken::Text("&".to_string())]] }
    }

    pub fn sixteenth() -> Self {
        Self {
            slots: vec![
                vec![LabelToken::BeatNum],
                vec![LabelToken::Text("e".to_string())],
                vec![LabelToken::Text("&".to_string())],
                vec![LabelToken::Text("a".to_string())],
            ],
        }
    }

    pub fn triplet() -> Self {
        Self {
            slots: vec![
                vec![LabelToken::BeatNum],
                vec![LabelToken::Text("trip".to_string())],
                vec![LabelToken::Text("let".to_string())],
            ],
        }
    }

    fn is_triplet(&self) -> bool {
        if self.slots.len() != 3 {
            return false;
        }
        matches!(self.slots.first().map(Vec::as_slice), Some([LabelToken::BeatNum]))
            && matches!(self.slots.get(1).map(Vec::as_slice), Some([LabelToken::Text(s)]) if s == "trip")
            && matches!(self.slots.get(2).map(Vec::as_slice), Some([LabelToken::Text(s)]) if s == "let")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Subdiv {
    Fixed(u8),
    TupletN,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CountScope {
    Measure,
    PrimaryGroup,
    BeatUnit,
    TupletAll,
    Tuplet(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountLayer {
    pub id: u32,
    pub enabled: bool,
    pub scope: CountScope,
    pub subdiv: Subdiv,
    pub labels: Option<LabelPattern>,
    pub show_labels: bool,
    pub colors: Option<ColorPattern>,
    pub show_colors: bool,
    pub priority: u8,
}

impl CountLayer {
    pub fn new(id: u32, scope: CountScope, subdiv: Subdiv) -> Self {
        Self {
            id,
            enabled: true,
            scope,
            subdiv,
            labels: None,
            show_labels: true,
            colors: None,
            show_colors: true,
            priority: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountConfig {
    pub layers: Vec<CountLayer>,
}

impl CountConfig {
    pub fn new(layers: Vec<CountLayer>) -> Self { Self { layers } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountSlot {
    pub layer_id: u32,
    pub scope: CountScope,
    pub scope_idx: u32,
    pub sub_idx: u8,
    pub start_tick: u32,
    pub end_tick: u32,
    pub label: Option<String>,
    pub color: Option<ColorId>,
    pub priority: u8,
    pub tuplet_id: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct Span {
    start_tick: u32,
    end_tick: u32,
    idx: u32,
}

#[derive(Clone, Copy, Debug)]
struct TupletSpan {
    id: u32,
    idx: u32,
    start_tick: u32,
    end_tick: u32,
    n: u8,
}

struct LabelContext {
    beat_num: u32,
    group_num: u32,
    sub_num: u8,
}

#[derive(Clone, Copy)]
struct ScopeSpan {
    scope: CountScope,
    scope_idx: u32,
    start_tick: u32,
    end_tick: u32,
    tuplet_id: Option<u32>,
    tuplet_n: Option<u8>,
}

impl ScopeSpan {
    fn from_span(scope: CountScope, span: Span) -> Self {
        Self {
            scope,
            scope_idx: span.idx,
            start_tick: span.start_tick,
            end_tick: span.end_tick,
            tuplet_id: None,
            tuplet_n: None,
        }
    }

    fn from_tuplet(span: TupletSpan) -> Self {
        Self {
            scope: CountScope::Tuplet(span.id),
            scope_idx: span.idx,
            start_tick: span.start_tick,
            end_tick: span.end_tick,
            tuplet_id: Some(span.id),
            tuplet_n: Some(span.n),
        }
    }
}

struct CountContext<'a> {
    slots: &'a mut Vec<CountSlot>,
    ticks_per_beat: u32,
    primary_groups: &'a [Span],
}

impl<'a> CountContext<'a> {
    fn push_slots(&mut self, layer: &CountLayer, span: ScopeSpan) {
        let subdiv = resolve_subdiv(layer.subdiv, span.tuplet_n);
        let span_ticks = span.end_tick.saturating_sub(span.start_tick);
        if subdiv == 0 || span_ticks == 0 {
            return;
        }
        let subdiv_u32 = subdiv as u32;
        if !span_ticks.is_multiple_of(subdiv_u32) {
            return;
        }
        let step = span_ticks / subdiv_u32;
        for sub_idx in 0..subdiv {
            let sub_idx_u32 = sub_idx as u32;
            let slot_start = span.start_tick + step * sub_idx_u32;
            let slot_end = slot_start + step;
            let beat_num = (slot_start / self.ticks_per_beat) + 1;
            let group_num = group_num_for_tick(self.primary_groups, slot_start);
            let ctx = LabelContext { beat_num, group_num, sub_num: sub_idx + 1 };
            let label = if layer.show_labels {
                layer.labels.as_ref().and_then(|p| {
                    let tuplet_kind = span.tuplet_n.map(TupletKind::from_n);
                    if p.is_triplet() && tuplet_kind.is_some_and(|k| k != TupletKind::Triplet) {
                        label_from_tokens(&[LabelToken::SubNum], &ctx)
                    } else {
                        label_for_slot(p, sub_idx, &ctx)
                    }
                })
            } else {
                None
            };
            let color = if layer.show_colors {
                layer.colors.as_ref().and_then(|p| color_for_slot(p, span.scope_idx, sub_idx))
            } else {
                None
            };
            self.slots.push(CountSlot {
                layer_id: layer.id,
                scope: span.scope,
                scope_idx: span.scope_idx,
                sub_idx,
                start_tick: slot_start,
                end_tick: slot_end,
                label,
                color,
                priority: layer.priority,
                tuplet_id: span.tuplet_id,
            });
        }
    }
}

pub fn build_count_slots(measure: &Measure, config: &CountConfig) -> Vec<CountSlot> {
    let ts = measure.time_signature();
    let ticks_per_beat = DEFAULT_GRID.ticks_per_beat(&ts);
    let measure_ticks = DEFAULT_GRID.ticks_per_measure(&ts);

    if ticks_per_beat == 0 || measure_ticks == 0 {
        return Vec::new();
    }

    let primary_groups = primary_group_spans(&ts, ticks_per_beat, measure_ticks);
    let beat_units = beat_unit_spans(&ts, ticks_per_beat);
    let tuplets = tuplet_spans(measure, measure_ticks);

    let mut slots = Vec::new();
    {
        let mut ctx =
            CountContext { slots: &mut slots, ticks_per_beat, primary_groups: &primary_groups };

        for layer in &config.layers {
            if !layer.enabled {
                continue;
            }
            match layer.scope {
                CountScope::Measure => {
                    let span = Span { start_tick: 0, end_tick: measure_ticks, idx: 0 };
                    ctx.push_slots(layer, ScopeSpan::from_span(CountScope::Measure, span));
                }
                CountScope::PrimaryGroup => {
                    for span in &primary_groups {
                        ctx.push_slots(
                            layer,
                            ScopeSpan::from_span(CountScope::PrimaryGroup, *span),
                        );
                    }
                }
                CountScope::BeatUnit => {
                    for span in &beat_units {
                        ctx.push_slots(layer, ScopeSpan::from_span(CountScope::BeatUnit, *span));
                    }
                }
                CountScope::TupletAll => {
                    for span in &tuplets {
                        ctx.push_slots(layer, ScopeSpan::from_tuplet(*span));
                    }
                }
                CountScope::Tuplet(id) => {
                    if let Some(span) = tuplets.iter().find(|s| s.id == id) {
                        ctx.push_slots(layer, ScopeSpan::from_tuplet(*span));
                    }
                }
            }
        }
    }

    slots.sort_by(|a, b| {
        a.start_tick
            .cmp(&b.start_tick)
            .then_with(|| b.priority.cmp(&a.priority))
            .then_with(|| a.layer_id.cmp(&b.layer_id))
    });
    slots
}

fn primary_group_spans(ts: &TimeSignature, ticks_per_beat: u32, measure_ticks: u32) -> Vec<Span> {
    let groups = default_groups_for(ts);
    let mut spans = Vec::new();
    let mut acc = 0u32;
    for (idx, g) in groups.iter().enumerate() {
        let start_tick = acc;
        let mut end_tick = acc + (*g as u32) * ticks_per_beat;
        if start_tick >= measure_ticks {
            break;
        }
        if end_tick > measure_ticks {
            end_tick = measure_ticks;
        }
        spans.push(Span { start_tick, end_tick, idx: idx as u32 });
        acc = end_tick;
    }
    spans
}

fn beat_unit_spans(ts: &TimeSignature, ticks_per_beat: u32) -> Vec<Span> {
    let mut spans = Vec::with_capacity(ts.beats as usize);
    for i in 0..ts.beats {
        let start_tick = (i as u32) * ticks_per_beat;
        let end_tick = start_tick + ticks_per_beat;
        spans.push(Span { start_tick, end_tick, idx: i as u32 });
    }
    spans
}

fn tuplet_spans(measure: &Measure, measure_ticks: u32) -> Vec<TupletSpan> {
    let beats = measure.beats();
    if beats.is_empty() {
        return Vec::new();
    }
    let onsets = DEFAULT_GRID.compute_onset_ticks(beats);
    measure
        .tuplet_groups()
        .into_iter()
        .filter_map(|group| measure.tuplet_anchors.get(&group.id).map(|a| (group, a)))
        .enumerate()
        .map(|(idx, (group, anchor))| {
            let start_tick = onsets[group.start_idx];
            let end_tick = (start_tick + anchor.target_ticks).min(measure_ticks);
            TupletSpan { id: group.id, idx: idx as u32, start_tick, end_tick, n: anchor.n }
        })
        .collect()
}

fn resolve_subdiv(subdiv: Subdiv, tuplet_n: Option<u8>) -> u8 {
    match subdiv {
        Subdiv::Fixed(n) => n,
        Subdiv::TupletN => tuplet_n.unwrap_or(0),
    }
}

fn group_num_for_tick(groups: &[Span], tick: u32) -> u32 {
    for g in groups {
        if tick >= g.start_tick && tick < g.end_tick {
            return g.idx + 1;
        }
    }
    1
}

fn label_for_slot(pattern: &LabelPattern, sub_idx: u8, ctx: &LabelContext) -> Option<String> {
    if pattern.slots.is_empty() {
        return None;
    }
    let slot = &pattern.slots[sub_idx as usize % pattern.slots.len()];
    label_from_tokens(slot, ctx)
}

fn label_from_tokens(tokens: &[LabelToken], ctx: &LabelContext) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    let mut out = String::new();
    for token in tokens {
        match token {
            LabelToken::BeatNum => out.push_str(&ctx.beat_num.to_string()),
            LabelToken::GroupNum => out.push_str(&ctx.group_num.to_string()),
            LabelToken::SubNum => out.push_str(&ctx.sub_num.to_string()),
            LabelToken::Text(s) => out.push_str(s),
        }
    }
    Some(out)
}

fn color_for_slot(pattern: &ColorPattern, scope_idx: u32, sub_idx: u8) -> Option<ColorId> {
    if pattern.palette.is_empty() {
        return None;
    }
    let idx = match pattern.mode {
        ColorMode::Scope => scope_idx as usize,
        ColorMode::Sub => sub_idx as usize,
    };
    Some(pattern.palette[idx % pattern.palette.len()])
}
