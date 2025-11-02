#![allow(dead_code)]

use eframe::egui::{Context, Rangef, Stroke, Ui};
use eframe::emath::{Align2, pos2};
use eframe::epaint::text::{FontInsert, InsertFontFamily};
use eframe::epaint::{Color32, FontFamily, FontId};
use eframe::{App, CreationContext, egui};
use egui::RichText;
use egui::containers::Frame;

struct MyApp {
    text: String,
    font_family: FontFamily,
    font_id: FontId,
}

fn add_font(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "Bravura",
        egui::FontData::from_static(include_bytes!("/usr/share/fonts/OTF/Bravura.otf")),
        vec![InsertFontFamily {
            family: FontFamily::Name("music".into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

enum Sticking {
    R, L
}

enum Duration {
    Quarter,
    Eighth,
    TripletEighth,
    Sixteenth,
    QuintupletSixteenth,
    SextupletSixteenth,
    SeptupletSixteenth,
    ThirtySecond,
    NonupletThirtySecond
}

struct Note {
    duration: Duration,
    sticking: Sticking,
}

struct Rest {
    duration: Duration
}

enum Beat {
    Note(Note),
    Rest(Rest)
}

impl MyApp {
    fn new(cc: &CreationContext) -> Self {
        add_font(&cc.egui_ctx);
        let ff = FontFamily::Name("music".into());
        Self {
            text: "Test".to_string(),
            font_family: ff.clone(),
            font_id: FontId::new(96.0, ff),
        }
    }

    // fn add(self, )
}

impl App for MyApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            Frame::canvas(ui.style()).show(ui, |ui| {
                let (_id, rect) = ui.allocate_space(ui.available_size());
                ui.painter().hline(
                    Rangef::new(rect.left(), rect.right()),
                    rect.center().y,
                    Stroke::new(1.0, Color32::WHITE),
                );
                // ui.painter().text(
                //     rect.min,
                //     Align2::LEFT_TOP,
                //     "",
                //     self.font_id.clone(),
                //     Color32::WHITE,
                // )
            });
        });
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([320.0, 240.0]),
        ..Default::default()
    };

    eframe::run_native(
        "My App",
        options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}
