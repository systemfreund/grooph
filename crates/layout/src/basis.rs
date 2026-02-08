use grooph_measure::duration::Duration;
use grooph_measure::Beat;
use grooph_measure::grid::DEFAULT_GRID;

pub fn calculate_x_centers(beats: &[Beat], content_w: f32, proportional: bool) -> Vec<f32> {
    let durations: Vec<Duration> = beats.iter().map(|b| b.duration).collect();

    // Calculate total duration in ticks
    let mut total_ticks = 0;
    let ticks_vec: Vec<u32> = durations.iter().map(|d| {
        let t = DEFAULT_GRID.ticks_of(d).unwrap_or(0);
        total_ticks += t;
        t
    }).collect();

    if !proportional {
        // Fallback or explicit uniform spacing
        let total = durations.len() as f32;
        let cell_w = if total > 0.0 { content_w / total } else { 1.0 };
        let mut x_centers = vec![0.0; durations.len()];
        let mut run = 0.0;
        for x_center in x_centers.iter_mut() {
            *x_center = run + cell_w * 0.5;
            run += cell_w;
        }
        return x_centers;
    }

    let px_per_tick = content_w / (total_ticks as f32);

    let mut x_centers: Vec<f32> = Vec::with_capacity(durations.len());
    let mut current_x = 0.0;

    for t in ticks_vec {
        let width = (t as f32) * px_per_tick;
        x_centers.push(current_x + width * 0.5);
        current_x += width;
    }

    x_centers
}

#[cfg(test)]
mod tests {
    use super::*;
    use grooph_measure::duration::{q, e};
    use grooph_measure::Beat;

    #[test]
    fn test_proportional_spacing() {
        let beats = vec![
            Beat::note(q()),
            Beat::note(e()),
            Beat::note(e()),
        ];
        let width = 100.0;
        let centers = calculate_x_centers(&beats, width, true);

        assert_eq!(centers.len(), 3);
        assert!((centers[0] - 25.0).abs() < 1e-5, "Expected 25.0, got {}", centers[0]);
        assert!((centers[1] - 62.5).abs() < 1e-5, "Expected 62.5, got {}", centers[1]);
        assert!((centers[2] - 87.5).abs() < 1e-5, "Expected 87.5, got {}", centers[2]);
    }

    #[test]
    fn test_uniform_spacing() {
        let beats = vec![
            Beat::note(q()),
            Beat::note(e()),
            Beat::note(e()),
        ];
        let width = 100.0;
        let centers = calculate_x_centers(&beats, width, false);

        assert_eq!(centers.len(), 3);
        // Uniform spacing: 3 items over 100.0 width.
        // Cell width = 100.0 / 3 = 33.33...
        // Centers: 16.66, 50.0, 83.33
        assert!((centers[0] - 16.66).abs() < 0.01, "Expected ~16.66, got {}", centers[0]);
        assert!((centers[1] - 50.00).abs() < 0.01, "Expected ~50.00, got {}", centers[1]);
        assert!((centers[2] - 83.33).abs() < 0.01, "Expected ~83.33, got {}", centers[2]);
    }
}
