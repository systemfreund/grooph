use crate::Grooph;
use crate::Mode;
use crate::generator::ENDLESS_MIN_BARS;
use crate::tool_palette::notation_button;
use eframe::egui;
use grooph_measure::TimeSignature;
use grooph_measure::generator::{
    BeatSpan, GroupingSet, MAX_BARS, MAX_COMPLEXITY, Subdivision, beat_groups,
    complexity_groupings, effective_subdivision, groupings, supports_time_signature,
};

/// Time signatures in the picker: every quarter-note and eighth-note meter
/// the editor offers (eighth-note meters are written in beat groups, e.g.
/// 6/8 = 3+3, 7/8 = 2+3+2).
fn time_signatures() -> Vec<TimeSignature> {
    [4, 8]
        .into_iter()
        .flat_map(|beat_unit| {
            TimeSignature::beat_range(beat_unit)
                .into_iter()
                .flatten()
                .map(move |beats| TimeSignature { beats, beat_unit })
        })
        .filter(|ts| supports_time_signature(*ts))
        .collect()
}

/// Beat groups shown next to an eighth-note meter, e.g. "7/8 (2+3+2)".
fn time_signature_label(ts: TimeSignature) -> String {
    let groups = beat_groups(ts).unwrap_or_default();
    if ts.beat_unit != 8 || groups.len() < 2 {
        return ts.to_string();
    }
    let sizes: Vec<&str> = groups
        .iter()
        .map(|g| match g {
            BeatSpan::Quarter => "2",
            BeatSpan::DottedQuarter => "3",
        })
        .collect();
    format!("{ts} ({})", sizes.join("+"))
}

fn subdivision_label(s: Subdivision) -> &'static str {
    match s {
        Subdivision::Eighths => "8th Notes",
        Subdivision::Sixteenths => "16th Notes",
        Subdivision::Triplets => "Triplets",
        Subdivision::Mixed => "Mixed",
    }
}

/// One labelled setting: caption above the control, like RhythmBot's header.
fn setting(ui: &mut egui::Ui, label: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.add(
            egui::Label::new(egui::RichText::new(label).weak())
                .wrap_mode(egui::TextWrapMode::Extend),
        );
        add_control(ui);
    });
}

impl Grooph {
    /// Generator settings (RhythmBot-style). Any change regenerates the score;
    /// "New rhythm" rolls again with unchanged settings.
    pub(super) fn generator_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("generator").resizable(true).show_animated_inside(
            ui,
            self.ui.mode == Mode::Generator,
            |ui| {
                let before = self.editor.generator.settings;
                let endless_before = self.editor.generator.endless;
                let mut roll = false;

                egui::ScrollArea::horizontal().id_salt("gen_settings_scroll").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("🎲 New").on_hover_text("Generate a new rhythm").clicked() {
                            roll = true;
                        }
                        ui.separator();

                        let gen_state = &mut self.editor.generator;
                        let settings = &mut gen_state.settings;

                        // Triplets and mixed have no x/8 cells; the generator
                        // falls back to 8ths / 16ths there.
                        let shown =
                            effective_subdivision(settings.subdivision, settings.time_signature);
                        let offered: &[Subdivision] = if settings.time_signature.beat_unit == 8 {
                            &[Subdivision::Eighths, Subdivision::Sixteenths]
                        } else {
                            &[
                                Subdivision::Eighths,
                                Subdivision::Sixteenths,
                                Subdivision::Triplets,
                                Subdivision::Mixed,
                            ]
                        };
                        setting(ui, "Subdivision", |ui| {
                            egui::ComboBox::from_id_salt("gen_subdivision")
                                .selected_text(subdivision_label(shown))
                                .show_ui(ui, |ui| {
                                    for &s in offered {
                                        ui.selectable_value(
                                            &mut settings.subdivision,
                                            s,
                                            subdivision_label(s),
                                        );
                                    }
                                });
                        });

                        let custom_on = settings.custom_groupings_enabled;
                        setting(ui, "Complexity", |ui| {
                            ui.add_enabled_ui(!custom_on, |ui| {
                                egui::ComboBox::from_id_salt("gen_complexity")
                                    .selected_text(format!("Level {}", settings.complexity))
                                    .show_ui(ui, |ui| {
                                        for level in 1..=MAX_COMPLEXITY {
                                            ui.selectable_value(
                                                &mut settings.complexity,
                                                level,
                                                format!("Level {level}"),
                                            );
                                        }
                                    });
                            })
                            .response
                            .on_disabled_hover_text("Custom groupings are active");
                        });

                        setting(ui, "Bars", |ui| {
                            let min_bars = if gen_state.endless { ENDLESS_MIN_BARS } else { 1 };
                            settings.bars = settings.bars.clamp(min_bars, MAX_BARS);
                            egui::ComboBox::from_id_salt("gen_bars")
                                .selected_text(settings.bars.to_string())
                                .show_ui(ui, |ui| {
                                    for bars in min_bars..=MAX_BARS {
                                        ui.selectable_value(
                                            &mut settings.bars,
                                            bars,
                                            bars.to_string(),
                                        );
                                    }
                                });
                        });

                        setting(ui, "Time Signature", |ui| {
                            egui::ComboBox::from_id_salt("gen_time_signature")
                                .selected_text(time_signature_label(settings.time_signature))
                                .show_ui(ui, |ui| {
                                    for ts in time_signatures() {
                                        ui.selectable_value(
                                            &mut settings.time_signature,
                                            ts,
                                            time_signature_label(ts),
                                        );
                                    }
                                });
                        });

                        setting(ui, "Space", |ui| {
                            ui.add(
                                egui::Slider::new(&mut settings.space, 0.0..=1.0).show_value(false),
                            )
                            .on_hover_text(format!("{:.0}%", settings.space * 100.0));
                        });

                        setting(ui, "Endless", |ui| {
                            ui.checkbox(&mut gen_state.endless, "").on_hover_text(
                                "Replace every bar with a new one right after it was played",
                            );
                        });

                        setting(ui, "Ghost Notes", |ui| {
                            ui.checkbox(&mut gen_state.ghost_notes, "").on_hover_text(
                                "Play quiet hits on every slot of the subdivision without a note",
                            );
                        });
                    });
                });

                egui::CollapsingHeader::new("Groupings")
                    .id_salt("gen_groupings")
                    .show(ui, |ui| self.groupings_picker(ui));

                let endless_turned_on = !endless_before && self.editor.generator.endless;
                let too_short_for_endless = self.editor.score.len() < ENDLESS_MIN_BARS;
                if roll
                    || self.editor.generator.settings != before
                    || (endless_turned_on && too_short_for_endless)
                {
                    self.generate_new_score();
                }
            },
        );
    }

    /// Grouping picker (reference app: "Custom Groupings"). While custom
    /// groupings are off, the tiles show which groupings the complexity level
    /// unlocks; picking a tile switches to a custom selection seeded from it.
    fn groupings_picker(&mut self, ui: &mut egui::Ui) {
        let family = self.ui.music_font_id.family.clone();
        let settings = &mut self.editor.generator.settings;
        let ts = settings.time_signature;
        let preset = complexity_groupings(settings.subdivision, ts, settings.complexity);

        let mut enabled = settings.custom_groupings_enabled;
        if ui.checkbox(&mut enabled, "Custom groupings").changed() {
            if enabled && settings.custom_groupings.is_empty() {
                settings.custom_groupings = preset;
            }
            settings.custom_groupings_enabled = enabled;
        }

        let active: GroupingSet =
            if settings.custom_groupings_enabled { settings.custom_groupings } else { preset };
        let available = groupings(settings.subdivision, ts);
        if settings.custom_groupings_enabled
            && !available.iter().any(|g| settings.custom_groupings.contains(g.id))
        {
            ui.weak("No grouping selected for this subdivision, using the complexity level.");
        } else if !settings.custom_groupings_enabled {
            ui.weak(format!(
                "Level {} uses the highlighted groupings. Pick one to customise.",
                settings.complexity
            ));
        }

        // Meters with both group sizes (5/8, 7/8) get one row per size.
        let mixed_spans = available.iter().any(|g| g.span != available[0].span);
        for span in [BeatSpan::Quarter, BeatSpan::DottedQuarter] {
            let of_span: Vec<_> = available.iter().filter(|g| g.span == span).collect();
            if of_span.is_empty() {
                continue;
            }
            if mixed_spans {
                ui.weak(match span {
                    BeatSpan::Quarter => "Groups of two eighths",
                    BeatSpan::DottedQuarter => "Groups of three eighths",
                });
            }
            ui.horizontal_wrapped(|ui| {
                for grouping in of_span {
                    let measure = grouping.measure();
                    let id = egui::Id::new(("grouping", grouping.id.0));
                    let tile = notation_button(
                        ui,
                        id,
                        &measure,
                        family.clone(),
                        active.contains(grouping.id),
                    )
                    .on_hover_text(format!("Unlocked at level {}", grouping.level));
                    if tile.clicked() {
                        if !settings.custom_groupings_enabled {
                            settings.custom_groupings = preset;
                            settings.custom_groupings_enabled = true;
                        }
                        settings.custom_groupings.toggle(grouping.id);
                    }
                }
            });
        }
    }
}
