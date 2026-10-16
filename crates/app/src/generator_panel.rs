use crate::Grooph;
use crate::Mode;
use crate::generator::ENDLESS_MIN_BARS;
use crate::pickers::Picker;
use crate::tool_palette::notation_button;
use eframe::egui;
use grooph_measure::TimeSignature;
use grooph_measure::generator::{
    BeatSpan, GeneratorSettings, GroupingSet, MAX_BARS, MAX_COMPLEXITY, Subdivision,
    complexity_groupings, effective_subdivision, groupings, offered_subdivisions,
    supports_time_signature,
};

/// Time signatures in the picker: every meter the editor offers in each of
/// its groupings (x/8 and x/16 are written in beat groups, e.g. 6/8 = 3+3,
/// 7/16 = 2+3+2 or 2+2+3).
pub(crate) fn time_signatures() -> Vec<TimeSignature> {
    TimeSignature::BEAT_UNITS
        .into_iter()
        .flat_map(|beat_unit| {
            TimeSignature::beat_range(beat_unit)
                .into_iter()
                .flatten()
                .map(move |beats| TimeSignature::new(beats, beat_unit))
        })
        .flat_map(|ts| (0..ts.groupings().len() as u8).map(move |g| ts.with_grouping(g)))
        .filter(|ts| supports_time_signature(*ts))
        .collect()
}

/// Beat groups shown next to an x/8 or x/16 meter, e.g. "7/8 (2+3+2)".
pub(crate) fn time_signature_label(ts: TimeSignature) -> String {
    let meter = format!("{}/{}", ts.beats, ts.beat_unit);
    if ts.beat_unit == 4 || ts.groups().len() < 2 {
        return meter;
    }
    format!("{meter} ({})", ts.groups_label())
}

pub(crate) fn subdivision_label(s: Subdivision) -> &'static str {
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
    /// the 🎲 button in the main menu rolls again with unchanged settings.
    pub(super) fn generator_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("generator").resizable(true).show_animated_inside(
            ui,
            self.ui.mode == Mode::Generator,
            |ui| {
                let before = self.editor.generator.settings;
                let endless_before = self.editor.generator.endless;
                let mut preset = None;

                egui::ScrollArea::horizontal().id_salt("gen_settings_scroll").show(ui, |ui| {
                    ui.horizontal(|ui| {
                        setting(ui, "Preset", |ui| preset = self.preset_combo(ui));

                        let gen_state = &mut self.editor.generator;
                        let settings = &mut gen_state.settings;

                        // Triplets and mixed have no x/8 cells; the generator
                        // falls back to 8ths / 16ths there. x/16 is always 16ths.
                        let shown =
                            effective_subdivision(settings.subdivision, settings.time_signature);
                        let offered = offered_subdivisions(settings.time_signature);
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

                        let mut pick_time_signature = false;
                        setting(ui, "Time Signature", |ui| {
                            // Same grid of meters and groupings as the compact layout.
                            pick_time_signature = ui
                                .button(format!(
                                    "{} ⏷",
                                    time_signature_label(settings.time_signature)
                                ))
                                .clicked();
                        });
                        if pick_time_signature {
                            self.ui.picker = Some(Picker::TimeSignature);
                        }

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

                egui::CollapsingHeader::new("Presets").id_salt("gen_presets").show(ui, |ui| {
                    if let Some(id) = self.preset_manager(ui) {
                        preset = Some(id);
                    }
                });

                self.finish_generator_change(before, endless_before, false);
                if let Some(id) = preset {
                    self.apply_preset(id);
                }
            },
        );
    }

    /// Regenerate the score after the generator settings were shown: on
    /// "New rhythm" (`roll`), on any changed setting, and when endless mode
    /// was turned on for a score too short for it.
    pub(crate) fn finish_generator_change(
        &mut self,
        before: GeneratorSettings,
        endless_before: bool,
        roll: bool,
    ) {
        let endless_turned_on = !endless_before && self.editor.generator.endless;
        let too_short_for_endless = self.editor.score.len() < ENDLESS_MIN_BARS;
        if roll
            || self.editor.generator.settings != before
            || (endless_turned_on && too_short_for_endless)
        {
            self.generate_new_score();
        }
    }

    /// Grouping picker (reference app: "Custom Groupings"). While custom
    /// groupings are off, the tiles show which groupings the complexity level
    /// unlocks; picking a tile switches to a custom selection seeded from it.
    pub(crate) fn groupings_picker(&mut self, ui: &mut egui::Ui) {
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
        for span in BeatSpan::ALL {
            let of_span: Vec<_> = available.iter().filter(|g| g.span == span).collect();
            if of_span.is_empty() {
                continue;
            }
            if mixed_spans {
                ui.weak(match span {
                    BeatSpan::Quarter => "Groups of two eighths",
                    BeatSpan::DottedQuarter => "Groups of three eighths",
                    BeatSpan::Eighth => "Groups of two sixteenths",
                    BeatSpan::DottedEighth => "Groups of three sixteenths",
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
