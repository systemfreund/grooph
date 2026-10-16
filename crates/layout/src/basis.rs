use grooph_measure::Beat;
use grooph_measure::duration::Duration;
use grooph_measure::grid::DEFAULT_GRID;

/// X position of each beat's note head, relative to the start of the note
/// area `content_w` wide.
///
/// Each note sits at its onset and its duration's space follows it, as in
/// printed music: the first note sits `lead` in from the left edge and the
/// last note's space runs up to the right edge. With `proportional`, space is
/// proportional to duration; otherwise every beat gets the same width.
///
/// `min_advance[i]` (px, optional per beat) is the least distance beat `i`
/// needs to the next note head or the barline so their glyphs don't touch.
/// Beats that would get less keep exactly that much; the rest of the width
/// is shared out by duration among the others.
pub fn calculate_x_centers(
    beats: &[Beat],
    content_w: f32,
    proportional: bool,
    lead: f32,
    min_advance: &[f32],
) -> Vec<f32> {
    let durations: Vec<Duration> = beats.iter().map(|b| b.duration).collect();
    let lead = lead.clamp(0.0, content_w.max(0.0));
    let body_w = content_w - lead;

    let weights: Vec<f32> = if proportional {
        durations.iter().map(|d| DEFAULT_GRID.ticks_of(d).unwrap_or(0) as f32).collect()
    } else {
        vec![1.0; durations.len()]
    };
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
        let centers = calculate_x_centers(&beats, width, true, 10.0, &[]);

        assert_eq!(centers.len(), 3);
        assert!((centers[0] - 10.0).abs() < 1e-5, "Expected 10.0, got {}", centers[0]);
        assert!((centers[1] - 55.0).abs() < 1e-5, "Expected 55.0, got {}", centers[1]);
        assert!((centers[2] - 77.5).abs() < 1e-5, "Expected 77.5, got {}", centers[2]);
    }

    #[test]
    fn test_uniform_spacing() {
        let beats = vec![Beat::note(q()), Beat::note(e()), Beat::note(e())];
        let width = 100.0;
        let centers = calculate_x_centers(&beats, width, false, 10.0, &[]);

        assert_eq!(centers.len(), 3);
        // Uniform spacing: 3 cells of 30 px after the 10 px lead.
        assert!((centers[0] - 10.0).abs() < 0.01, "Expected 10.0, got {}", centers[0]);
        assert!((centers[1] - 40.0).abs() < 0.01, "Expected 40.0, got {}", centers[1]);
        assert!((centers[2] - 70.0).abs() < 0.01, "Expected 70.0, got {}", centers[2]);
    }

    #[test]
    fn crowded_beats_keep_their_minimum_advance() {
        // Quarter, then two sixteenths that each need 20 px. Proportionally
        // they'd get 100/6 px each; they keep 20 and the quarter takes the
        // rest.
        use grooph_measure::duration::s;
        let beats = vec![Beat::note(q()), Beat::note(s()), Beat::note(s())];
        let centers = calculate_x_centers(&beats, 100.0, true, 0.0, &[0.0, 20.0, 20.0]);
        assert!((centers[0] - 0.0).abs() < 1e-4);
        assert!((centers[1] - 60.0).abs() < 1e-4, "got {}", centers[1]);
        assert!((centers[2] - 80.0).abs() < 1e-4, "got {}", centers[2]);
    }

    #[test]
    fn uncrowded_beats_stay_proportional() {
        let beats = vec![Beat::note(q()), Beat::note(e()), Beat::note(e())];
        let plain = calculate_x_centers(&beats, 100.0, true, 0.0, &[]);
        let with_needs = calculate_x_centers(&beats, 100.0, true, 0.0, &[5.0, 5.0, 5.0]);
        assert_eq!(plain, with_needs);
    }
}
