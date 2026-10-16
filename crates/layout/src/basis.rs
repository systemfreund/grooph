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
pub fn calculate_x_centers(
    beats: &[Beat],
    content_w: f32,
    proportional: bool,
    lead: f32,
) -> Vec<f32> {
    let durations: Vec<Duration> = beats.iter().map(|b| b.duration).collect();
    let lead = lead.clamp(0.0, content_w.max(0.0));
    let body_w = content_w - lead;

    if !proportional {
        let total = durations.len() as f32;
        let cell_w = if total > 0.0 { body_w / total } else { 1.0 };
        return (0..durations.len()).map(|i| lead + cell_w * i as f32).collect();
    }

    let ticks_vec: Vec<u32> =
        durations.iter().map(|d| DEFAULT_GRID.ticks_of(d).unwrap_or(0)).collect();
    let total_ticks: u32 = ticks_vec.iter().sum();
    let px_per_tick = if total_ticks > 0 { body_w / total_ticks as f32 } else { 0.0 };

    let mut x_centers: Vec<f32> = Vec::with_capacity(durations.len());
    let mut current_x = lead;
    for t in ticks_vec {
        x_centers.push(current_x);
        current_x += t as f32 * px_per_tick;
    }
    x_centers
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
        let centers = calculate_x_centers(&beats, width, true, 10.0);

        assert_eq!(centers.len(), 3);
        assert!((centers[0] - 10.0).abs() < 1e-5, "Expected 10.0, got {}", centers[0]);
        assert!((centers[1] - 55.0).abs() < 1e-5, "Expected 55.0, got {}", centers[1]);
        assert!((centers[2] - 77.5).abs() < 1e-5, "Expected 77.5, got {}", centers[2]);
    }

    #[test]
    fn test_uniform_spacing() {
        let beats = vec![Beat::note(q()), Beat::note(e()), Beat::note(e())];
        let width = 100.0;
        let centers = calculate_x_centers(&beats, width, false, 10.0);

        assert_eq!(centers.len(), 3);
        // Uniform spacing: 3 cells of 30 px after the 10 px lead.
        assert!((centers[0] - 10.0).abs() < 0.01, "Expected 10.0, got {}", centers[0]);
        assert!((centers[1] - 40.0).abs() < 0.01, "Expected 40.0, got {}", centers[1]);
        assert!((centers[2] - 70.0).abs() < 0.01, "Expected 70.0, got {}", centers[2]);
    }
}
