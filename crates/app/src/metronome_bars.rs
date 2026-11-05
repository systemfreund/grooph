//! The metronome's bars of every meter, kept in its document alone.
//!
//! Each meter has a click bar, optionally a counter rhythm, and the ratio
//! that rhythm was picked as ([`MeterBars`]). The metronome's document
//! ([`MetronomeDoc`]) owns them: the meter it clicks is its score, the one
//! bar the staff and the tools edit, and every other meter's are in its
//! [`MetronomeBars`]. Both are in the document's undo snapshot, so a meter
//! change, a click edit and a ratio pick undo as one.
//!
//! The score is private to this module. The meter changes only through
//! [`MetronomeDoc::click_meter`] and [`MetronomeDoc::keep_meter_bars`], which
//! keep the old meter's bars; every other edit goes through
//! [`Doc::edit_score_by`], which refuses one that changes the meter.

use crate::documents::Doc;
use crate::metronome_view::{MetronomeVoice, metronome_bar};
use crate::undo::{DEFAULT_UNDO_LIMIT, EditorSnapshot, UndoHistory};
use grooph_measure::click::{ClickBars, meter_click_bar};
use grooph_measure::counter::{Ratio, named_ratio_of, respelled};
use grooph_measure::{Cursor, Measure, Score, TimeSignature};
use serde::{Deserialize, Serialize};

/// One meter's bars.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct MeterBars {
    /// The click bar, with the counter rhythm, if any, as its second voice:
    /// the bar the metronome's document holds.
    pub(crate) bar: Measure,
    /// The ratio the counter rhythm was picked as, where its bar is also
    /// another's (4:2 plays 2:1's bar).
    pub(crate) name: Option<Ratio>,
}

impl MeterBars {
    /// `ts`'s own clicks and no counter rhythm.
    pub(crate) fn plain(ts: TimeSignature) -> Self {
        Self { bar: meter_click_bar(ts), name: None }
    }

    /// `click` with `counter`, if any, picked as `name`.
    pub(crate) fn new(click: Measure, counter: Option<&Measure>, name: Option<Ratio>) -> Self {
        Self { bar: metronome_bar(click, counter), name }
    }

    pub(crate) fn meter(&self) -> TimeSignature {
        self.bar.time_signature()
    }

    pub(crate) fn click(&self) -> Measure {
        self.bar.single_voice(MetronomeVoice::Click.idx()).expect("a measure's first voice")
    }

    pub(crate) fn counter(&self) -> Option<Measure> {
        self.bar.single_voice(MetronomeVoice::Counter.idx())
    }

    /// The name only while the counter rhythm is that ratio's, as kept for
    /// a meter not clicked.
    fn tidied(self) -> Self {
        let counter = self.counter();
        let name = self
            .name
            .filter(|&r| counter.as_ref().is_some_and(|c| named_ratio_of(c, Some(r)) == Some(r)));
        Self { name, ..self }
    }

    /// The counter rhythm in today's spelling (see [`respelled`]), for bars
    /// saved before a spelling changed.
    fn respelled(self) -> Self {
        match self.counter() {
            Some(counter) => Self::new(self.click(), Some(&respelled(counter)), self.name),
            None => self,
        }
    }

    fn is_plain(&self) -> bool {
        *self == Self::plain(self.meter())
    }
}

/// The part of the metronome's document besides its score: the ratio name
/// of the meter it clicks, whose bar is the score, and every other meter's
/// bars. Empty in the other documents' undo snapshots.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MetronomeBars {
    /// The ratio the score's counter rhythm was picked as.
    name: Option<Ratio>,
    /// The other meters' bars that are not their plain ones, one per meter.
    others: Vec<MeterBars>,
}

impl MetronomeBars {
    fn other(&self, ts: TimeSignature) -> Option<&MeterBars> {
        self.others.iter().find(|b| b.meter() == ts)
    }

    fn take(&mut self, ts: TimeSignature) -> Option<MeterBars> {
        let idx = self.others.iter().position(|b| b.meter() == ts)?;
        Some(self.others.remove(idx))
    }

    fn keep(&mut self, bars: MeterBars) {
        self.take(bars.meter());
        let bars = bars.tidied();
        if !bars.is_plain() {
            self.others.push(bars);
        }
    }
}

/// The metronome's document: the click bar of the meter it clicks, as a
/// one-bar score, and every other meter's bars. It has no saved pattern and
/// no unsaved mark.
pub(crate) struct MetronomeDoc {
    score: Score,
    cursor: Cursor,
    history: UndoHistory,
    meters: MetronomeBars,
}

impl MetronomeDoc {
    /// A document clicking `bars`, with nothing kept and nothing to undo.
    fn new(bars: MeterBars) -> Self {
        Self {
            score: Score::single(bars.bar),
            cursor: Cursor::start(),
            history: UndoHistory::new(DEFAULT_UNDO_LIMIT),
            meters: MetronomeBars { name: bars.name, others: Vec::new() },
        }
    }

    /// The meter it clicks.
    pub(crate) fn meter(&self) -> TimeSignature {
        self.score.measures()[0].time_signature()
    }

    /// The ratio the clicked meter's counter rhythm was picked as.
    pub(crate) fn counter_name(&self) -> Option<Ratio> {
        self.meters.name
    }

    /// The metronome document's bars of the meter it clicks.
    pub(crate) fn clicked_bars(&self) -> MeterBars {
        MeterBars { bar: self.score.measures()[0].clone(), name: self.meters.name }
    }

    /// The metronome document's bars of `ts`: the clicked meter's, else
    /// those kept, else its plain ones.
    pub(crate) fn meter_bars(&self, ts: TimeSignature) -> MeterBars {
        if ts == self.meter() {
            return self.clicked_bars();
        }
        self.meters.other(ts).cloned().unwrap_or_else(|| MeterBars::plain(ts))
    }

    /// The metronome's document clicks `bars` from now on, on their first
    /// click; the meter clicked before keeps its own.
    pub(crate) fn click_meter(&mut self, bars: MeterBars) {
        let before = self.clicked_bars();
        if before.meter() != bars.meter() {
            self.meters.keep(before);
        }
        self.meters.take(bars.meter());
        self.meters.name = bars.name;
        self.score = Score::single(bars.bar);
        self.cursor = Cursor::start();
    }

    /// `bars` are their meter's from now on, clicked or not; the meter
    /// clicked stays, with the cursor where it was.
    pub(crate) fn keep_meter_bars(&mut self, bars: MeterBars) {
        if bars.meter() != self.clicked_bars().meter() {
            self.meters.keep(bars);
            return;
        }
        self.meters.name = bars.name;
        self.score = Score::single(bars.bar);
        self.cursor = self.score.clamp_cursor(self.cursor);
    }

    /// The click bar of every meter, for the audio engine.
    pub(crate) fn click_bars(&self) -> ClickBars {
        let mut bars = ClickBars::default();
        for other in &self.meters.others {
            bars.set(other.click());
        }
        bars.set(self.clicked_bars().click());
        bars
    }
}

impl Doc for MetronomeDoc {
    fn score(&self) -> &Score {
        &self.score
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn cursor_mut(&mut self) -> &mut Cursor {
        &mut self.cursor
    }

    fn history(&self) -> &UndoHistory {
        &self.history
    }

    fn history_mut(&mut self) -> &mut UndoHistory {
        &mut self.history
    }

    fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            score: self.score.clone(),
            cursor: self.cursor,
            meters: self.meters.clone(),
        }
    }

    fn restore(&mut self, snap: EditorSnapshot) {
        self.score = snap.score;
        self.cursor = snap.cursor;
        self.meters = snap.meters;
    }

    /// An edit that would change the meter is taken back: the meter
    /// changes only through [`MetronomeDoc::click_meter`] and
    /// [`MetronomeDoc::keep_meter_bars`], which keep the old meter's bars.
    fn edit_score_by(&mut self, edit: &mut dyn FnMut(&mut Score)) {
        let before = self.score.clone();
        edit(&mut self.score);
        let meter = before.measures()[0].time_signature();
        if self.score.measures().first().map(Measure::time_signature) != Some(meter) {
            log::warn!("an edit would have changed the metronome's meter; taken back");
            self.score = before;
            self.cursor = self.score.clamp_cursor(self.cursor);
        }
    }
}

/// The metronome's bars as saved: the meter it clicks and every meter's
/// bars that are not its plain ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SavedMetronome {
    pub(crate) meter: TimeSignature,
    pub(crate) bars: Vec<MeterBars>,
}

impl Default for SavedMetronome {
    fn default() -> Self {
        Self { meter: TimeSignature::FOUR_FOUR, bars: Vec::new() }
    }
}

impl SavedMetronome {
    pub(crate) fn of(doc: &MetronomeDoc) -> Self {
        let clicked = doc.clicked_bars();
        let bars = std::iter::once(clicked.clone())
            .filter(|b| !b.is_plain())
            .chain(doc.meters.others.iter().cloned())
            .collect();
        Self { meter: clicked.meter(), bars }
    }

    /// The metronome's document again, with nothing to undo. Of a meter
    /// saved twice the last counts.
    pub(crate) fn into_doc(self) -> MetronomeDoc {
        let mut doc = MetronomeDoc::new(MeterBars::plain(self.meter));
        let mut clicked = MeterBars::plain(self.meter);
        for bars in self.bars.into_iter().map(MeterBars::respelled) {
            if bars.meter() == self.meter {
                clicked = bars;
            } else {
                doc.meters.keep(bars);
            }
        }
        doc.keep_meter_bars(clicked);
        doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::click::ClickLevel;
    use grooph_measure::counter::counter_bar;

    #[test]
    fn another_meter_keeps_a_name_while_its_bar_is_that_ratios() {
        let ts: TimeSignature = "2/4".parse().unwrap();
        let (four_two, three_two) = (Ratio { n: 4, span: 2 }, Ratio { n: 3, span: 2 });
        // 4:2 plays 2:1's bar; with its clicks edited it is still 4:2.
        let mut edited = counter_bar(ts, four_two).unwrap();
        edited.first_voice_mut().set_click_level(1, ClickLevel::Strong);
        let mut bars = MetronomeBars::default();
        bars.keep(MeterBars::new(meter_click_bar(ts), Some(&edited), Some(four_two)));
        assert_eq!(bars.other(ts).unwrap().name, Some(four_two));
        // Another ratio's bar drops the name.
        let other = counter_bar(ts, three_two).unwrap();
        bars.keep(MeterBars::new(meter_click_bar(ts), Some(&other), Some(four_two)));
        assert_eq!(bars.other(ts).unwrap().name, None);
        // Plain bars are not kept.
        bars.keep(MeterBars::plain(ts));
        assert_eq!(bars, MetronomeBars::default());
    }

    /// An edit of the score keeps the meter: one that would change it is
    /// taken back, so the new meter's kept bars are not hidden by it.
    #[test]
    fn an_edit_cannot_change_the_meter() {
        let (four, three) = (TimeSignature::FOUR_FOUR, TimeSignature::THREE_FOUR);
        let mut edited = meter_click_bar(three);
        edited.first_voice_mut().set_click_level(0, ClickLevel::Off);
        let mut doc = MetronomeDoc::new(MeterBars::plain(three));
        doc.keep_meter_bars(MeterBars::new(edited.clone(), None, None));
        doc.click_meter(MeterBars::plain(four));
        let doc: &mut dyn Doc = &mut doc;
        // A beat edit stays.
        doc.edit_score(|s| {
            s.measures_mut()[0].first_voice_mut().set_click_level(1, ClickLevel::Off)
        });
        let clicked = doc.score().clone();
        assert_ne!(clicked, Score::single(meter_click_bar(four)));
        // A meter change, by a bar's meter or by a whole score, does not.
        let set = doc.edit_score(|s| s.measures_mut()[0].set_time_signature(three));
        assert!(set.is_ok());
        assert_eq!(*doc.score(), clicked);
        doc.edit_score(|s| *s = Score::single(meter_click_bar(three)));
        assert_eq!(*doc.score(), clicked);
        // 3/4 kept its edited bar, and comes back with it through
        // `click_meter`.
        let snap = doc.snapshot();
        let mut doc = MetronomeDoc::new(MeterBars::plain(four));
        doc.restore(snap);
        doc.click_meter(doc.meter_bars(three));
        assert_eq!(doc.clicked_bars().click(), edited);
    }
}
