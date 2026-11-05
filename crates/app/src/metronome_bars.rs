//! The metronome's bars of every meter, kept in its document alone.
//!
//! Each meter has a click bar, optionally a counter rhythm, and the ratio
//! that rhythm was picked as ([`MeterBars`]). The metronome's document
//! ([`RhythmDoc`]) owns them: the meter it clicks is its score, the one bar
//! the staff and the tools edit, and every other meter's are in its
//! [`MetronomeBars`]. Both are in the document's undo snapshot, so a meter
//! change, a click edit and a ratio pick undo as one. The meter changes only
//! through [`RhythmDoc::click_meter`], which keeps the old meter's bars.

use crate::documents::RhythmDoc;
use crate::metronome_view::{MetronomeVoice, metronome_bar};
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
/// bars. Empty in the other documents.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MetronomeBars {
    /// The ratio the score's counter rhythm was picked as.
    pub(crate) name: Option<Ratio>,
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

impl RhythmDoc {
    /// The metronome document's bars of the meter it clicks.
    pub(crate) fn clicked_bars(&self) -> MeterBars {
        let bar = self.score.measures()[0].clone();
        // A bar of a kept meter written into the score directly, not through
        // `click_meter`, would hide that meter's kept bars.
        debug_assert!(
            self.meters.other(bar.time_signature()).is_none(),
            "the metronome's score turned to a kept meter without click_meter"
        );
        MeterBars { bar, name: self.meters.name }
    }

    /// The metronome document's bars of `ts`: the clicked meter's, else
    /// those kept, else its plain ones.
    pub(crate) fn meter_bars(&self, ts: TimeSignature) -> MeterBars {
        if ts == self.score.measures()[0].time_signature() {
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
    pub(crate) fn of(doc: &RhythmDoc) -> Self {
        let clicked = doc.clicked_bars();
        let bars = std::iter::once(clicked.clone())
            .filter(|b| !b.is_plain())
            .chain(doc.meters.others.iter().cloned())
            .collect();
        Self { meter: clicked.meter(), bars }
    }

    /// The metronome's document again, with nothing to undo. Of a meter
    /// saved twice the last counts.
    pub(crate) fn into_doc(self) -> RhythmDoc {
        let mut doc = RhythmDoc::new(Score::single(meter_click_bar(self.meter)), Cursor::start());
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
}
