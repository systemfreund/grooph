use grooph_measure::Beat;
use grooph_measure::duration::Duration;
use grooph_measure::grid::DEFAULT_GRID;

/// X position of each beat's note head, relative to the start of the note
/// area `content_w` wide.
///
/// Each note sits at its onset and its duration's space follows it, as in
/// printed music: the first note sits `lead` in from the left edge and the
/// last note's space runs up to the right edge. Space is proportional to
/// duration.
///
/// `min_advance[i]` (px, optional per beat) is the least distance beat `i`
/// needs to the next note head or the barline so their glyphs don't touch.
/// Beats that would get less keep exactly that much; the rest of the width
/// is shared out by duration among the others.
pub fn calculate_x_centers(
    beats: &[Beat],
    content_w: f32,
    lead: f32,
    min_advance: &[f32],
) -> Vec<f32> {
    let durations: Vec<Duration> = beats.iter().map(|b| b.duration).collect();
    let lead = lead.clamp(0.0, content_w.max(0.0));
    let body_w = content_w - lead;

    let weights: Vec<f32> =
        durations.iter().map(|d| DEFAULT_GRID.ticks_of(d).unwrap_or(0) as f32).collect();
    let needs: Vec<f32> =
        (0..durations.len()).map(|i| min_advance.get(i).copied().unwrap_or(0.0)).collect();
    let advances = share_out(&weights, &needs, body_w);

    let mut x_centers: Vec<f32> = Vec::with_capacity(durations.len());
    let mut current_x = lead;
    for a in advances {
        x_centers.push(current_x);
        current_x += a;
    }
    x_centers
}

/// The shared columns of voices played together: one per distinct onset,
/// running to the next onset in any voice (the last to the barline). A
/// beat's least advance is spread over the columns it spans by their weight,
/// so each beat keeps at least its own advance.
struct Columns {
    /// Start tick of each column.
    onsets: Vec<u32>,
    weights: Vec<f32>,
    needs: Vec<f32>,
}

impl Columns {
    fn new(voices: &[(&[Beat], &[f32])]) -> Self {
        let ticks = |b: &Beat| DEFAULT_GRID.ticks_of(&b.duration).unwrap_or(0);
        let starts: Vec<Vec<u32>> =
            voices.iter().map(|(beats, _)| DEFAULT_GRID.compute_onset_ticks(beats)).collect();
        let end = voices
            .iter()
            .zip(&starts)
            .filter_map(|((beats, _), s)| Some(s.last()? + ticks(beats.last()?)))
            .max()
            .unwrap_or(0);
        let mut onsets: Vec<u32> = starts.iter().flatten().copied().collect();
        onsets.sort_unstable();
        onsets.dedup();
        let weights: Vec<f32> = (0..onsets.len())
            .map(|j| (onsets.get(j + 1).copied().unwrap_or(end) - onsets[j]) as f32)
            .collect();
        let mut needs = vec![0.0f32; onsets.len()];
        for ((_, min_advance), starts) in voices.iter().zip(&starts) {
            for (i, &start) in starts.iter().enumerate() {
                let stop = starts.get(i + 1).copied().unwrap_or(end);
                let need = min_advance.get(i).copied().unwrap_or(0.0);
                let spanned =
                    onsets.partition_point(|&t| t < start)..onsets.partition_point(|&t| t < stop);
                let weight: f32 = weights[spanned.clone()].iter().sum();
                if weight <= 0.0 {
                    continue;
                }
                for j in spanned {
                    needs[j] = needs[j].max(need * weights[j] / weight);
                }
            }
        }
        Columns { onsets, weights, needs }
    }
}

/// [`calculate_x_centers`] for voices played together, each with its own
/// `min_advance`: beats at the same time, in any voice, share their x, and
/// space is shared out over the columns between successive onsets.
pub fn calculate_shared_x_centers(
    voices: &[(&[Beat], &[f32])],
    content_w: f32,
    lead: f32,
) -> Vec<Vec<f32>> {
    let columns = Columns::new(voices);
    let lead = lead.clamp(0.0, content_w.max(0.0));
    let advances = share_out(&columns.weights, &columns.needs, content_w - lead);
    let xs: Vec<f32> = advances
        .iter()
        .scan(lead, |x, a| {
            let at = *x;
            *x += a;
            Some(at)
        })
        .collect();
    voices
        .iter()
        .map(|(beats, _)| {
            DEFAULT_GRID
                .compute_onset_ticks(beats)
                .iter()
                .map(|t| xs[columns.onsets.partition_point(|o| o < t)])
                .collect()
        })
        .collect()
}

/// The least width of the columns of [`calculate_shared_x_centers`], lead
/// excluded: at it, every beat gets just its `min_advance`.
pub fn shared_min_width(voices: &[(&[Beat], &[f32])]) -> f32 {
    Columns::new(voices).needs.iter().sum()
}

/// Split `total` into `max(needs[i], weights[i] * k)` per item, with `k` as
/// large as fits. Items whose weighted share falls below their need get
/// their need; the others share what is left by weight. When the needs alone
/// exceed `total`, every item gets its need.
fn share_out(weights: &[f32], needs: &[f32], total: f32) -> Vec<f32> {
    let mut fixed = vec![false; weights.len()];
    let mut k = 0.0;
    // Each round fixes at least one more item at its need, or settles.
    for _ in 0..=weights.len() {
        let fixed_sum: f32 = needs.iter().zip(&fixed).filter(|(_, f)| **f).map(|(n, _)| n).sum();
        let free_weight: f32 =
            weights.iter().zip(&fixed).filter(|(_, f)| !**f).map(|(w, _)| w).sum();
        k = if free_weight > 0.0 { ((total - fixed_sum) / free_weight).max(0.0) } else { 0.0 };
        let mut changed = false;
        for i in 0..weights.len() {
            if !fixed[i] && weights[i] * k < needs[i] {
                fixed[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    weights.iter().zip(needs).map(|(w, n)| (w * k).max(*n)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::Beat;
    use grooph_measure::duration::{e, q};

    #[test]
    fn test_proportional_spacing() {
        let beats = vec![Beat::note(q()), Beat::note(e()), Beat::note(e())];
        let width = 100.0;
        // 10 px lead, then 90 px split as quarter / eighth / eighth.
        let centers = calculate_x_centers(&beats, width, 10.0, &[]);

        assert_eq!(centers.len(), 3);
        assert!((centers[0] - 10.0).abs() < 1e-5, "Expected 10.0, got {}", centers[0]);
        assert!((centers[1] - 55.0).abs() < 1e-5, "Expected 55.0, got {}", centers[1]);
        assert!((centers[2] - 77.5).abs() < 1e-5, "Expected 77.5, got {}", centers[2]);
    }

    #[test]
    fn crowded_beats_keep_their_minimum_advance() {
        // Quarter, then two sixteenths that each need 20 px. Proportionally
        // they'd get 100/6 px each; they keep 20 and the quarter takes the
        // rest.
        use grooph_measure::duration::s;
        let beats = vec![Beat::note(q()), Beat::note(s()), Beat::note(s())];
        let centers = calculate_x_centers(&beats, 100.0, 0.0, &[0.0, 20.0, 20.0]);
        assert!((centers[0] - 0.0).abs() < 1e-4);
        assert!((centers[1] - 60.0).abs() < 1e-4, "got {}", centers[1]);
        assert!((centers[2] - 80.0).abs() < 1e-4, "got {}", centers[2]);
    }

    #[test]
    fn voices_spaced_together_share_their_onsets_and_keep_their_room() {
        use grooph_measure::duration::{h, th};
        // Sixteen 32nds and a half against two halves: the 32nds need 10 px
        // each, more than their share of the first half.
        let mut upper = vec![Beat::note(th()); 16];
        upper.push(Beat::note(h()));
        let lower = vec![Beat::note(h()), Beat::note(h())];
        let needs = vec![10.0; 16];
        let xs = calculate_shared_x_centers(&[(&upper, &needs), (&lower, &[])], 200.0, 0.0);
        // Both halves at the second half of the measure start together.
        assert!((xs[0][16] - xs[1][1]).abs() < 1e-4, "{} vs {}", xs[0][16], xs[1][1]);
        assert!((xs[0][0] - xs[1][0]).abs() < 1e-4);
        for w in xs[0][..17].windows(2) {
            assert!(w[1] - w[0] >= 10.0 - 1e-4, "{:?}", xs[0]);
        }
        assert!((shared_min_width(&[(&upper, &needs), (&lower, &[])]) - 160.0).abs() < 1e-3);
        // A voice alone is spaced as by itself.
        let alone = calculate_shared_x_centers(&[(&upper, &needs)], 200.0, 5.0);
        assert_eq!(alone[0], calculate_x_centers(&upper, 200.0, 5.0, &needs));
    }

    #[test]
    fn uncrowded_beats_stay_proportional() {
        let beats = vec![Beat::note(q()), Beat::note(e()), Beat::note(e())];
        let plain = calculate_x_centers(&beats, 100.0, 0.0, &[]);
        let with_needs = calculate_x_centers(&beats, 100.0, 0.0, &[5.0, 5.0, 5.0]);
        assert_eq!(plain, with_needs);
    }
}
