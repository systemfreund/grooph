mod beam_plan;
pub(crate) mod render_plan;
pub(crate) mod pixel_layout;

use crate::measure::Measure;
use crate::measure::duration::Duration;

pub(crate) fn calculate_x_centers(measure: &Measure, content_w: f32) -> Vec<f32> {
    let durations: Vec<Duration> = measure.beats().iter().map(|b| b.duration).collect();

    // Normalize to fit the content box
    let total: f32 = durations.len() as f32;
    let cell_w = if total > 0.0 { content_w / total } else { 1.0 };

    let mut x_centers: Vec<f32> = vec![0.0; durations.len()];
    let mut run = 0.0_f32;
    for x_center in x_centers.iter_mut() {
        *x_center = run + cell_w * 0.5;
        run += cell_w;
    }
    x_centers
}
