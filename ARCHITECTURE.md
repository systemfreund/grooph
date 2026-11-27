# grooph architecture

Key concepts, files and invariants of the codebase, so changes can be made
with confidence. Build, test and review practices are in AGENTS.md; the
user-facing overview is README.md.

## Project map (workspace)

- `crates/app`: GUI, app state (`Grooph`), panels, tool palette, input, persistence.
- `crates/measure`: Domain model for measures/beats/duration, editing logic, counting.
- `crates/layout`: Pixel layout from the model (note positions, beam/tuplet plans).
- `crates/render`: Draws the layout with egui (notes, beams, tuplets, cursor).
- `crates/audio`: Metronome synth (rodio), scheduling based on the measure.
- `crates/midi`: MIDI input abstraction (midir).
- `crates/clock`: The app's monotonic clock (`performance.now()` on the web) and `OffsetTracker`, which maps a device's
  timestamps onto it; shared by `app`, `midi` and `mic` so every hit lives on one timeline.
- `crates/mic`: Microphone hit detection (cpal natively, `getUserMedia` + AudioWorklet on the web).
- `crates/link`: Shareable links (URL query parameters <-> score/generator/metronome).
- `crates/site`: Static guide pages for grooph.app, generated from Markdown at build time.
- `crates/link-check`: grooph-link's parser as a WebAssembly module for the MCP server's link check.
- `functions/`, `mcp/`: the MCP server (`grooph.app/mcp`) and the view that embeds the metronome in AI chats.
- `crates/android`: Android entry point (`android_main`, NativeActivity) around `crates/app`.

Important root files:
- `Cargo.toml` (workspace + dependencies, edition 2024)
- `Trunk.toml` (WASM build/serve)
- `rustfmt.toml` (formatting)

## Crate dependencies

Dependencies point one way, from the app down to the model:

- `measure` is the base: the music model and its rules, with no GUI, audio, device or browser dependencies.
- `layout`, `audio` and `link` build on `measure`; `render` on `layout` and `measure`; `site` on `link` and
  `measure`. `layout` takes geometry types and text measurement from egui but paints nothing; drawing is
  `render`'s job. `audio` and `link` know nothing about the GUI.
- `midi` and `mic` stand alone: they deliver input events and know nothing about the model. Both read `clock`, the
  app's clock (MIDI counts from when its `MidiInput` opened), and the app moves their hits onto its timeline by
  their age, so a MIDI hit, a microphone hit and a tap compare directly.
- `app` ties everything together: state, panels, persistence, undo. `android` only wraps `app`.

Music rules (durations, onsets, grouping, counting) belong in `measure`, not in `app` or a panel, so the
click, the notation, the counting and the practice tracker share them. Platform differences sit behind
`cfg(target_arch = "wasm32")` inside the crate that talks to the platform (`audio`, `mic`, `midi`, `app`), not
spread across callers.

## Data model and invariants

- Core object: `grooph_measure::Measure`, a `TimeSignature` and its voices
  (`VoiceBar`, at least one): each voice a `Vec<Beat>` filling the meter, with
  its own tuplet groups. Voice 0 has its stems up; a second voice sits below
  the shared line with its stems down (the metronome's counter rhythm under
  the click bar; later the feet under the hands of a drum part). All voices
  share the meter: `Measure::set_time_signature` refits every one, or none.
  Each voice keeps the meter for itself (it is filled, edited and laid out
  on its own), but only `grooph_measure::voice`, where `Measure` and
  `VoiceBar` live, can set it: no voice leaves its measure's meter.
  A `Measure` names the voice it is read or edited in: `first_voice()` is
  the upper one, the only one of click bars, the generator, counting and
  the audio's click; `voices()`, `voice(i)`, `voice_mut(i)` and
  `push_voices` reach the rest, and `time_signature()` is the meter of all.
  There is no `Deref` to the first voice, so code that forgets a lower
  voice does not compile by accident. `Beat::is_kit` and
  `Score::has_kit_heads` tell notes of the kit from the one line's.
  `Score` keeps its measures private: `measures()`/`measures_mut()` give
  the slice, and `push`, `insert`, `remove` and `truncate` keep at least
  one measure. Measures saved before voices (beats and tuplets inline) load as one
  voice (`RawMeasure`).
- `Beat` is a slot of the rhythm, `duration` and `tuplet` (its place in a tuplet group), and what
  is played in it, its `Heads` (`grooph_measure::head`): none is a rest, one a
  note, several a chord on one stem (at most one per `Instrument`, sorted from
  the foot up, at most `MAX_HEADS`). Each `Head` has its `Instrument`,
  `accented`, `soft` (a ghost note, head in parentheses; never with
  `accented`: the setters clear the other flag, a save with both keeps the
  accent) and a `Technique` its instrument allows (open hi-hat,
  cross-stick, ...; checked when built or loaded). `kind()`, `accented()`,
  `soft()` read the beat as a whole (any head); `set_accented`/`set_soft` set
  every head, `set_kind_of` turns a rest into a plain note of an instrument
  and back (`set_kind`: the one line's `Rhythm`).
  On the one line every note is one `Instrument::Rhythm` head;
  `head::REGISTER` describes each instrument once (`InstrumentSpec`:
  notation code, labels, staff position and notehead of the drumkit legend
  for kit scores (#251-#255), techniques with their notation letters, mixer
  group, and whether a hand or a foot plays it, `Limb`); `ALL`, `staff_pos`, `notehead`, `techniques`, the notation, the
  palette labels and `KitVolumes::of` read from it; `Score::voicing` (`Voicing`, the set of
  instruments entered in the lower voice) says which voice a new head goes to when the measure does not say otherwise (`Measure::voice_for`). Beats saved before heads (`kind`,
  `accented`, `soft`) load as one `Rhythm` head (`RawBeat`).
  The editing operations of a voice take the instrument they mean (see
  "Kit editing"): accent and ghost change its head, `toggle_head` adds or
  removes it, `cycle_technique` steps through its techniques, and a rest
  made a note (a duration, Enter, a tuplet) gets its head. A tuplet keeps
  the notes it is made of: a new group's slots play plain heads of its
  note's instruments (`Heads::plain`), another tuplet or a dissolved one
  moves each note's heads as they are.
- Sticking (`grooph_measure::sticking`, #348): a beat with a hand instrument
  (`Instrument::is_hand`; the one line's `Rhythm` too, not the kick or the
  hi-hat's foot) may carry one `Sticking` (R, L or both) for all its heads,
  a field of the `Beat` like its tuplet mark, so it moves with its note when
  notes before it are inserted, deleted or split. Any voice can carry them.
  `Beat::set_heads` drops it with the last hand instrument; `set_beat`
  (`carry_accents_from`), a tuplet made, changed or dissolved, Clean up and
  `VoiceBar::from_onsets` keep it on the note they keep. `Measure::stickings`
  gives one per column (tick) across the voices: the same one stays, different
  ones or both hands are both (written R above L).
  A sticking button that writes one moves the cursor on to the next beat
  of its voice in the bar that takes one (`Grooph::step_cursor`, as a note
  or rest moves on to the next beat), so R L R R … is tapped in a row; one
  taken away, and accent, ghost, dot and techniques, stay on their beat.
- `Duration` is `Simple`, `Dotted`, or `Tuplet(TupletSpec { n, m, base })`.
- `Measure::set_beat` guarantees a valid measure length and fills gaps.
- `DEFAULT_GRID` defines the valid duration grid and tick calculations.
- Tuplets:
  - A tuplet beat automatically creates the remaining beats of the group.
  - Groups are positions, not names (`grooph_measure::tuplet`): the first beat carries
    `TupletMark::Start { base }`, the others `TupletMark::Continue`. The ratio comes from the beats'
    `TupletSpec`, the span is `m` of the group's base (`TupletGroup::span_ticks`), which can differ from
    the first beat's value (`t4 t8` in a triplet of eighths). `tuplet_group_at`/`tuplet_groups` read
    them as `TupletGroup { start_idx, end_idx, spec }`. Voices saved before marks (`tuplet_group_id` on
    the beats, `tuplet_anchors` beside them) are converted on load (`OldAnchors` in `voice.rs`).
  - Loading checks whole groups: every tuplet in a group, two or more beats of one ratio that fill the
    span, no mark outside a group (`check_tuplet_groups`).
  - `set_beat` can absorb/fill following beats to keep the measure consistent.
- Generator (`grooph_measure::generator`): builds random measures for
  sight-reading from per-beat figures (onset patterns, see Groupings). `GeneratorSettings` = subdivision,
  complexity (1-5, caps the figure level), bars, time signature (x/4, x/8,
  x/16), space (extra whole-beat rests). In x/8 and x/16 the generator beats
  are the meter's beat groups (`beat_groups`, from `default_groups_for`:
  6/8 = 3+3, 7/8 = 2+3+2, the same for x/16). Two-eighth groups use the
  quarter-beat figures, three-eighth groups (`BeatSpan::DottedQuarter`) and
  the x/16 groups (`Eighth`, `DottedEighth`) the patterns of their own span. Triplets/mixed fall back to
  8ths/16ths in x/8, x/16 is always 16ths (`effective_subdivision`). A figure
  only fixes the note onsets; `generator/spelling.rs` then picks each note's
  length at random (up to the next onset or shorter, early stops inside a beat from
  level 4) and `grooph_measure::spelling` writes notes and rests for the whole measure from
  the onsets (on a thirty-second grid): notes cross beats
  only as one value without a tie (dotted quarter or half from a beat, quarter
  or dotted quarter from the "and") inside a half bar (x/4 beats grouped like
  the eighths of n/8) or as a half centred on its boundary (`4 2 4`), never
  out of an x/8 or x/16 beat group; a note from the "and" over a beat only
  with eighths around it (no sixteenths in the beats it starts and ends in,
  the two-level rhythmic parsing rule); triplet figures
  are written as drawn. Measures are written via `set_beat`, so
  they obey the same invariants as edited ones. Seed `Rng` for reproducibility.
- Revoicing (`grooph_measure::revoice`, #300): `revoice(measure, voices, target)` gives each head
  (`head_onsets`) a voice and writes every voice again: heads at one tick in one voice are a chord,
  accent, ghost, technique and sticking stay. A voice of the measure that sounds on exactly the
  new ticks keeps its beats and tuplets (only the heads change), so the author's spelling survives;
  otherwise the voice is written from its onsets and respelled as Clean up does. A head of a tuplet
  brings its group along (rests where the voice does not play); groups of two voices over the same
  time meet in one of their ratio (`merged_group`). `None` where tuplets cannot meet (triplets
  and quintuplets over one beat) or a chord would get more than `MAX_HEADS`; the tools then leave
  the bar alone. `Measure::revoiced(voicing)` sorts by a preset ("Revoice bar…" in the bar menu,
  `Picker::Revoice`, tool `measure.revoice`), `Measure::with_voice_switched` moves one instrument
  to the other voice, every head in the bar or the one at the cursor (`modify.voice.swap`,
  Shift+V; `modify.voice.swap_one`, V). A lower voice left empty goes in a score of one voice.
  Each is one undo step; the cursor stays at its time, in the voice its instrument went to.
  The kit's tool grid ends with both after the meter (`VOICE_TOOLS`); "Revoice bar…" stays in the
  bar menu. A bar "Revoice bar…" cannot revoice stays, and
  a short notice says so (`Grooph::notify`, `notice.rs`, about 3 s).
- The score's rule (`app`'s `voicing_picker`, #301): the score tab's "Voicing" row opens
  `Picker::Voicing`, a tile per preset with the cursor's bar revoiced by it (`voicing_preview`; a
  bar without notes shows a 2/4 example) and "Custom", a switch per instrument (`KIT_ORDER`,
  `Voicing::with_lower`). The picker edits a pending rule (`Dialogs::voicing_edit`); when it closes
  changed, and revoicing would change a bar with notes, "Revoice existing measures too?" asks
  (`Dialogs::voicing_prompt`): Yes sets the rule and revoices every bar, No the rule only, each one
  undo step (`Grooph::set_voicing`); Cancel or dismissing it keeps the old rule. Otherwise the rule
  is set without asking. A new drum groove gets the rule of the setting "Write new grooves as"
  (`EditorSettings::new_groove_voicing`, a settings group of its own; "Use this score's" copies the
  score tab's rule); scores from links and the library derive theirs from their content.
- Clean up (`grooph_measure::cleanup`, the bar menu's "Clean up bar"):
  `VoiceBar::respelled` writes a voice again from its onsets with the same
  spelling rules, deterministically: a drum hit does not sustain, so only
  where each note starts (with its heads) is the rhythm. Each note lasts
  up to the next one but only to the end of its beat (a backbeat reads
  `4-rest 4 4-rest 4`), rests merge per beat and into half rests at the
  start of a pair of beats, `x/8` and `x/16` keep to their beat groups as
  picked. Tuplets stay as written with the beats they share; a tuplet of
  rests only becomes a plain rest, and one whose notes all start on the
  plain grid (a triplet with only its first note) is respelled as plain
  notes; a group over several beats becomes one group per beat where a
  note of it starts on each beat, each beat holds two or more of its
  notes and the beat is `m` of a note value
  (subdivided triplet halves read as triplets of eighths, beat by beat),
  and a tuplet is written on the coarsest places its notes allow, in its
  ratio's lowest terms (a sextuplet sounding on every other note is a
  triplet of eighths); inside a tuplet notes last up to the next one and
  rests merge, as far as one value of its ratio lasts and within the
  beat (`t8 r:t8 t8` is `t4 t8`); a voice whose tuplet starts or ends
  inside a beat where no beat starts stays as it is (`None`). Clean up
  writes only tuplets the editor can enter: values of its value buttons
  (`editing::VALUE_BUTTONS`) in the group's own ratio or one of its
  tuplet tools (`editing::TUPLET_TOOLS`); a test keeps both lists equal
  to the palette's, so a new button widens Clean up too.
  `Measure::clean_up` does every voice; applying it twice changes
  nothing. `VoiceBar::from_onsets` writes a voice from ticks and heads
  alone, for moving heads between voices. An empty bar is per-beat rests in
  the model; the whole rest is the layout's (`RenderPlan::bar_rest`), also in
  the editor and, on the kit's staff, for an empty voice among several (in its half; not on a shared line).
  `Score::hidden_rests` (`HiddenRests`, #322; the link's `hiderests`) leaves out the rests of the lower
  voice or of both in the kit's bars of two voices, an empty voice's whole rest too (`RenderPlan::rests`):
  hidden, they keep their layouts for the labels, the cursor and taps; the editor draws them veiled
  (`StaffOpts::veil_hidden_rests`, `NoteLayout::veiled`). Rests of the kit's two voices are full size and
  placed by `staff_layout::clear_rests`: a staff space off where a voice alone has them toward their voice's
  side (in the middle with the other voice's rests hidden), then on a staff space at a time while within
  `REST_GAP_SS` of the other voice's ink, as Gould displaces rests only as far as needed. A bar of two empty voices has one whole rest for both (the upper voice's,
  where a voice alone has it; the lower voice's beats are hidden), as Gould writes a rest the voices share. The
  score tab's Rests row picks it.
- `Beat`, `VoiceBar` and `Measure` compare with `==` including their tuplet groups. Tests that build
  expected beats by hand (without marks) compare `beat::unmarked(..)`.

## Rendering pipeline

1. `grooph_layout::pixel_layout::build_measure_layout` computes `MeasureLayout`
   (note/beam/tuplet positions) from `Measure` and `LayoutOpts`.
2. `grooph_render` renders the result with egui (`staff::draw_staff`, one
   measure each with `measure::render_measure_at`). What a staff is drawn
   from (score, layout, options, cursor, playback marks) is one
   `staff::RenderContext`; each layer takes what it needs from it.
3. The font `Bravura.otf` (SMuFL) lives in `crates/app/assets/fonts`.
4. `grooph_layout::staff_layout::build_staff_layout` lays out a score, one
   measure per row, every voice of a measure (`PlacedMeasure::voices`, one
   `MeasureLayout` each; `layout()` is the first). With several voices they
   share the line and are spaced together (`calculate_shared_x_centers`):
   beats at the same time line up and each voice keeps the room its glyphs
   need. Nothing moves a note after spacing, or the voices would part.
   On the one line a rest under a beam is then a stemlet, clear of the
   line; on the kit's staff it stays a rest, as drum notation writes it.
   `StaffOpts::voice_opts_for` gives each voice its stem direction.
   The metronome tab edits one such measure: the click bar with the counter
   rhythm as its second voice. The cursor's voice is the bar being edited
   (`MetronomeVoice` names the two), as in a kit score; it is the one
   source of both (see "Metronome tab").
   The edit cursor (`Cursor`) names its measure, voice and beat. The tools
   edit that voice (`Score::voice_at_mut`); a tap picks the voice on its
   side of the line (`hit_test_voice`), on the kit's staff the head by its
   instrument's place (`hit_test_head`); moving to a measure with fewer
   voices puts it in the last (`Score::clamp_cursor`). A new measure gets
   the voices of the one before (`Measure::empty_like`), "clear" empties
   the cursor's voice only (`Measure::clear_voice`), and a new duration
   keeps a kit note's heads.
5. A score is written on one line or on the kit's five (`Score::staff`,
   `StaffKind`; the notation puts a score with kit instruments on the kit's
   staff). On the kit's staff (`LayoutOpts::staff`) each head sits at its
   instrument's place (`Instrument::staff_pos`), with its glyph
   (`glyphs::head_glyph_for`: x for cymbals, circled x for a cross-stick,
   slashed for a rimshot, diamond for the ride's bell). A chord's heads
   share one stem from the head at its base to past the one at its tip
   (`NoteLayout::head`, at `staff_pos`; the others in `chord`); a head a
   second from the one before it goes to the other side of the stem. Heads
   past the staff get ledger lines, rests keep to their voice's half, a
   beamed group's stems all reach its beams (`beams::reach_beams`). Past
   the accent a technique's sign (o, +, the choke's mark) stacks
   (`notes::place_marks`). Every kit row starts with the percussion clef.
   Rows of text along the staff (`grooph_layout::text_rows`): the count
   labels and the stickings, each above or below the staff or off
   (`StaffOpts::text_rows`, from the app's view settings `CountingSettings`
   and `StickingSettings`, not the score or its links; no stickings in the
   metronome). `build_staff_layout` places them per bar
   (`PlacedMeasure::text_rows`) clear of its notes' ink (accents, technique
   marks, the crash's head) and the staff's middle; on one side the
   sticking sits next to the notes and the count outside it. A bar's
   sticking row is two lines high where a column shows both hands (R
   above L there; single letters stay on the line next to the notes), one
   where it has stickings, none without; every row of the staff gets the room of the tallest
   (`RowRoom`). `grooph_render` draws them at their lines.
6. Kit editing (`grooph-app`'s `kit_palette`): on a staff of several
   instruments the tool grid's first row has a toggle per instrument
   (`Grooph::tap_instrument`): it adds the instrument's head to the cursor's
   beat, played the usual way, in the voice where it already is there, else
   where it is elsewhere in the measure, else voice 0 of a measure of one
   voice that holds notes, else the voice the score's `Voicing` writes it in
   (`Measure::voice_for`; a kit measure without that voice gets it,
   `Measure::fill_voices`), at the cursor's time: where a beat of that
   voice starts then, on it; else `Measure::start_note` (`strike.rs`)
   splits the rest or note sounding then, the note keeping its heads, marks
   and sticking, and the new note starts at that time. The span from the
   start of what was split to the end of the cursor's beat is written by
   Clean up's rules (no ties, rests merged per beat), the rest of the voice
   keeps its spelling; a time inside a tuplet of the cursor's voice gets
   that tuplet over the same span, its slots rests but the new note's,
   merged as Clean up does. Where nothing fits (triplets against
   sixteenths, another tuplet there) the bar stays and a notice says so.
   The tap, a voice the bar gets and the split are one undo step, and the
   cursor goes onto the new note. A tap on a head starting there plays it with the
   instrument's next technique (`Instrument::techniques`), after the last it
   takes it away. A note made a rest so, or by the key H, leaves its beat
   written again as Clean up writes it (`VoiceBar::tidy_beat_of`: the note
   before it lasts to the next one again, a tuplet of rests is a rest), and
   the cursor keeps its time: in its voice where a beat starts then, else
   in the lowest numbered other voice where one does, else on what sounds
   then. Only heads starting at the cursor's time are at the cursor
   (`heads_at_cursor`: the lit cells, the technique stepped, where marks go,
   the long press), not notes that started earlier and still sound; a long press (a right click, `Grooph::hold_instrument`)
   moves the instrument's note at the cursor to the other voice
   (`switch_voice(true)`, without falling back to another head; without
   its head on the beat it only picks the instrument). The row is always the nine cells
   (`grid_rows`, `KIT_ORDER`), so none move; each shows the head on the beat
   as on the staff, with the technique's sign (`paint_instrument`), faded
   where there is none, its short name in bold (drawn twice, a pixel apart:
   egui's font has no bold face) and a dot per technique, the one played
   lit. The cell shows as picked while the head starts at the cursor's
   time (`heads_at_cursor`). The instrument tapped, or of a head tapped on the
   staff, is the picked one (`EditorState::instrument`; `Grooph::edit_instrument`
   gives it, or the staff's own on the one line, `Staff::instrument_or`):
   the note values write it on a rest. Marks go to the head they mean
   (`Grooph::aimed`): a technique tool (`Modifier::ToggleTechnique`, not in
   the grid) plays the head of an instrument played that way on the beat
   so, or plainly again, so no instrument needs picking first;
   accent and ghost go to the picked instrument's head if it is on the beat,
   else the snare, kick, toms, hi-hat, ride, crash (`AIM_ORDER`), which
   becomes the picked one. The picked instrument's cell is always ringed
   (what the note values write, even on a rest), brighter where it is the
   head several on the beat aim at. O steps through the techniques.
   The library's "New drum groove" starts one bar of two voices on the
   kit's staff, asking first if the score has unsaved changes
   (`PendingLoad::NewKitScore`); it is offered while the score tab is
   shown. That tab is not finished: the tab bar hides it, with its
   "BETA" pill, until the developer setting `ViewSettings::show_score_tab`
   shows it, but a link or pattern still opens it. A drum is struck by one voice at a time:
   a tool whose edit makes two voices strike an instrument of the kit at
   the same tick is refused (`Measure::doubled_instruments`, compared
   before and after each tool in `apply_tool`, so a doubling the score
   already has does not block other edits). A score loaded from a link
   or a saved pattern is made so first (a link has no `Voicing`, so
   `replace_score_from_link` sets it from the content first,
   `Score::derive_voicing`; `Score::undouble_instruments`):
   a doubled head stays in the voice the score's `Voicing` writes its
   instrument in, its accent going along, and leaves the other, and the score is marked unsaved; so is a
   score restored from the saved session. The one line edits as before:
   its pen is `Rhythm`, and the kit's tools do nothing there.
7. Both staves are one model, `grooph_layout::staff::Staff`, which
   `Staff::of(StaffKind)` gives (`LayoutOpts::staff_model`; the only match
   on the kind). A staff holds its lines as staff positions (half staff
   spaces up from the middle line), the instruments it writes
   (`instruments`, `instrument_near` by place), where their heads sit, where
   its rests go, and whether every row starts with the clef. From that
   alone come the head and rest positions (`head_pos`, `rest_pos`,
   `bar_rest_pos`), the ledger lines of heads and half and whole rests
   past the outer lines (`ledgers`), the dots moved up off a line
   (`on_line`), the side of the line two voices keep to (`shares_line`,
   only on a staff of one line, where beamed rests become stemlets), the
   row's room for the lines, and the lines the renderer draws.

## Audio, playback, counting

- `grooph_audio::Audio` builds a playback schedule from the measure
  (Downbeat/Primary/Accent/Beat) and uses `DEFAULT_GRID`.
- A measure sounds as all its voices together (`grooph_measure::onsets`):
  `Measure::head_onsets` lists every head of every voice by tick, for the
  schedule; `Measure::onsets` every tick where a beat of any voice starts,
  once, a note if any voice plays there, with the beat that stands for it
  (the note of the lowest numbered voice playing there, else the rest of the
  lowest numbered voice with a beat there). The accuracy
  reference (`compute_global_beat_onsets`) and ghost notes follow `onsets`;
  the playback cursor, guide anchors, count labels, stickings and accuracy
  marks too, at the head of the beat standing for each onset: each bar's
  `TickMap` (`grooph_layout::tick_map`, `PlacedMeasure::ticks`) is the one
  mapping from a tick to x for all of them. Picking a stretch of time, a bar
  or a row on the staff is `hit_test_span`, `hit_test_bar` and
  `hit_test_row` next to `hit_test_head`.
  The layout's shared spacing of voices (`basis::Columns`) takes its ticks
  from the same `onsets::merged`, so drawing and sound agree on them.
  A one-line score has one voice, so its onsets are its beats.
- The playback cursor is smoothed in the UI, audio offset for latency is optional.
- Guide (`crates/app/src/guide.rs`, Practice options): what the staff shows
  while playing. `Line` is the playback cursor; the other steps hide it and
  only flash anchors on beats (the primary beats of each bar's meter): every
  beat, every second beat (from beat 1 in each bar; in two-beat meters the
  same as every bar), beat 1 of every bar or of every second bar, or nothing.
  Anchors mark the beat, not a note, so they light rests and held notes too.
  Without the line the count overlay lights its labels only with the anchors
  (`PlaybackMarks::Anchor`), not along the way.
  Bars are counted as played (`PlaybackState::bars_played`), so loops and
  endless mode keep the every-second-bar rhythm. The metronome and edit
  tabs and calibration always use the line. Not part of links.
- Counting overlay comes from `grooph_measure::counting`.
- Swing (`grooph_measure::swing::Swing`, global, main menu next to BPM): 50%
  (off) to 75% (dotted), swung unit 8ths or 16ths. Notation stays straight;
  `ScoreTiming::with_swing` carries it. `performed_global_tick` moves notes and
  ghost notes in the schedule and is the reference for accuracy hits/misses;
  `written_global_tick` maps the audio position back so the cursor reaches a
  swung note when it sounds. Pairs start at the downbeat, tuplet onsets and a
  trailing incomplete pair stay straight. Not stored with library patterns.
- Loop range (`grooph_measure::BarRange`, #384): the bars `a..=b` of Practice's
  or the score tab's document that play and loop; without one the whole score
  does. Measure indices and global ticks stay the score's, and so does the
  schedule; `PlaybackOptions::range` carries the range and `ScoreTiming::with_loop`
  makes it the timing's loop: `total_loop_ticks`/`total_loop_seconds` are the
  range's, `loop_start_tick`/`loop_end_tick` its ends, `seconds_to_global_tick`
  counts from `a`'s downbeat and goes round the range (`wrap_tick`), and
  `score_ticks` is the whole score's. `TickSource::restart` starts at `a` (the
  count-in in `a`'s meter and tempo, `count_in_plan`); at every barline the
  next bar is the one after (bar 1 after the last) if it is in the range, else
  `a` (`BarRange::next_bar`), so a range changed while playing takes over at
  the next barline, without a pause or count-in, playing on into it where it
  can. A plan with only a new range leaves the cursor alone. `bars()` counts
  the bars played, so the gap click counts them in the range and a ramp in
  loops steps once per pass (`TickSource::loop_len`, the range's bars, a new
  range's from the barline it takes over at). The app does the same
  with its cursor (`playback_clock::advance_tick`, `next_bar_tick`) and keeps
  the range still played until then (`loop_range::LoopPlayback::held`) for its
  timing (`Grooph::played_range`; the audio gets the document's,
  `audio_range`); when the cursor crosses the barline it reports
  `ScoreChange::Range` (`cross_barline`); a tempo change keeps the old range
  until then. The output latency takes the position heard back over a
  barline into the bar the cursor is still in (`playback_clock::heard_tick`),
  and the count-in ends once `a`'s downbeat is heard: timed from when the
  engine left it (`CountInState::Heard`), as a loop may be shorter than the
  latency. The app's ramp reads the loop length from the engine
  (`BarState::loop_len`). None in the metronome, a
  calibration, endless mode or the embedded view.
- The loop button (🔁, `Grooph::toggle_loop`) sits between play and share in
  both transport rows and next to play in the editor's bar, filled in the
  accent colour while a range is set; it switches the range off, and on again
  with the one last switched off (`RhythmDoc::parked_range`, moved by edits
  with its bars, saved as `parked_range`, forgotten by another score and
  endless mode), else with all bars. On the staff
  (`loop_range::loop_flags`, after the marks, before `staff_input`) a flag in
  the accent colour stands on the barline before `a` and after `b`, drawn over
  the space above the row; its grab area (`grab_rect`, 44 pt) over the top of
  its barline drags it (`StaffView::loop_drag`), snapping to the nearest
  barline of the row under the finger (`nearest_bar`), one bar before the other
  flag at most; letting go sets the range (`set_loop_range`). A press there
  never reaches `staff_input`, and the bar menu's chip moves clear of a flag.
  Bars outside the range get the gap click's veil without 🔇
  (`draw_outside_bar`), never the gap click's own, in the view and the editor
  (still editable); the ghost bar stays as it is.

## Text notation and shareable links

- `grooph_measure::notation`: `parse_score` / `format_score` for a compact
  text notation (`4/4 q e e >s s s s t8 t8 r:t8 | q. e r:q q`). Duration
  codes match the helpers in `duration.rs`; `r:` = rest, `>` = accent, `(e)` =
  soft note, `|` =
  bar line, a leading `N/D` sets the time signature. Parsing goes through
  `set_beat`; each measure must be filled exactly, errors name the measure
  and token. Formatting returns `None` for durations without a code.
  Drum kit notes name their heads before the duration: `S:e`, a chord
  `K-H:e` (`-`, not `+`, which links read as a space), per head `>S` and
  `(S)`, a technique as a lowercase letter (`Ho`, `Hc`, `Sx`, `RDb`, `CRc`);
  instruments `K S H HF RD CR T1 T2 FT` (`InstrumentSpec::code`; cymbals take two
  letters so `R` never reads as `r:`). A sticking follows a note after `@` (`q@R`,
  `(e)@L`, `S-H:e@RL` for both hands), only on a note played by hand; rhythm links
  keep the `@` unencoded (`percent_encode`; a click bar has no stickings). `;` separates the voices of a
  measure. Errors name the place (`Place`: measure and voice). A score with
  no note of the kit on the kit's staff (an empty kit bar) starts with
  `kit`; `format_score` writes it only then.
- App links carry drum kit scores in `r` (`grooph_link::is_kit`: the kit's
  staff, its instruments or several voices). `click` and `counter` stay on
  the one line. The MCP server is the metronome only: its link check
  (`grooph_link_check::open_query`) refuses a kit score, and its tool
  description and `llms.txt` do not describe the kit notation.
- `grooph_link` (`crates/link`): query parameters `bpm`, `r`, `ts`,
  `groups` (the grouping of `ts`, `ts=7/8&groups=3+2+2`; `ts` takes no
  grouping of its own), `swing`, `su` and the generator's `sub`, `lvl`, `bars`, `space`, `seed` (seeded
  `Rng`, so a link reproduces the same exercise). `parse_query` reads them,
  `SharedLink::to_url` writes them. The app reads the link once at startup
  (`Grooph::open_link_from_url`), then removes it from the address bar. A
  `ts` link on its own opens the Metronome tab with that meter (and its
  `click` bar) and leaves the rhythm alone. A polyrhythm is `poly=n:m`: the
  link builds the ratio's counter bar as the picker does (`counter_bar`) and
  names it; `counter=` carries the clicks written out, which the share
  buttons write only when they were edited (`SharedLink::poly`). `llms.txt`
  and the MCP tool offer only `poly`: models get the notation wrong too often. A score link replaces the score
  tab's score (and opens that tab) through `PendingLoad::Link` and the
  unsaved-changes dialog; a generator link, or a score link of the one
  line without `p=score`, replaces Practice's rhythm without asking; so
  does any score link while the score tab is hidden (`opens_on_practice`).
  `p=score` (`SharedLink::score_tab`, only with `r`) says the score was
  shared from the score tab, which a rhythm of the one line opens on too;
  links without it, all those from before, keep the rule by the staff. The
  share buttons (`share_link`) build a link with `SharedLink::to_url` and
  write `p=score` on the score tab.
- The link format is documented for AI agents in
  `crates/app/assets/seo/llms.txt`; keep it in sync.

## Embed mode and the MCP server

grooph's metronome runs inside AI chats as an
[MCP App](https://modelcontextprotocol.io/extensions/apps/overview):

- `functions/mcp.js` is a Cloudflare Pages Function, so `wrangler pages
  deploy dist` (run from the repository root) serves it as `/mcp` next to
  the web build: `https://grooph.app/mcp`, and each preview its own. The
  logic is in `mcp/server.js`: a stateless MCP server (Streamable HTTP, JSON
  answers, CORS) with one tool, `show_metronome`, whose arguments are the
  metronome's link parameters (`bpm`, `ts`, `groups`, `subdivision`,
  `click`, `poly`, `swing`, `su`): the app's own controls, with `click` only
  for bars the subdivision picker does not offer. A view cannot be reached
  by a later tool call (each call renders its own), so a change is a new
  call with `link`, the shown metronome's link: `resolveSettings` in
  `mcp/link.js` takes that link's settings and changes only what the other
  arguments set (a new meter drops groups, click, subdivision and
  polyrhythm; `click: "default"`, `poly: "none"`, `swing: 50` remove one),
  in the server and the view alike; the result names the changes. The old
  view stays in the chat; views of the server share the host's sandbox
  origin, so a view that starts playing tells the others on a
  `BroadcastChannel` and a playing one stops: one metronome plays at a
  time, across chats and tabs of the same browser. `mcp/link.js`
  builds the same query as `SharedLink::to_url`, checked by both test suites
  against `mcp/link-cases.json`. Its result carries the link, for hosts
  without views, and what the metronome shows (`grooph_link::describe`:
  meter and grouping, tempo, click bar with its strong, medium and soft
  clicks, polyrhythm, swing), so the model can check it against the request;
  the embedded app's model context starts with the same description. Before
  it answers, the tool checks the link with the app's own parser:
  `crates/link-check` builds `grooph_link::parse_query` and `describe` as a
  plain WebAssembly module (four exports, no imports, no wasm-bindgen; a
  Trunk hook writes it to `dist/embed/link-check.wasm`, which
  `functions/mcp.js` imports), loaded by `mcp/link-check.js`. Wrong notation or a `poly` that does not fit the
  meter comes back as `isError` with the app's message.
- The tool's view `ui://grooph/metronome.html` (`mcp/view.js`) runs in the
  host's sandboxed iframe on another origin. It does the MCP Apps handshake
  over postMessage, puts the link plus `embed=mcp&theme=light|dark` into the
  address bar, defines `window.groophHost` (`appUrl`, `theme`,
  `updateModelContext`, `openLink`) and imports the web build from the
  request's own origin (allowed by the resource's `_meta.ui.csp`). The
  hashed file names come from `embed.json`, which `crates/site` writes next
  to `index.html`, together with `mcp/link.js` as `/embed/link.js`, which
  the view imports to build the link. Never inline function source into the
  view (`fn.toString()`): Cloudflare bundles the function with esbuild,
  which adds helpers such as `__name` the view does not have. `_headers`
  sends CORS for `/grooph-app-*` and `/embed/*`. A host whose
  CSP forbids WebAssembly gets a message with the grooph.app link instead.
- Embed mode (`crates/app/src/embed.rs`): `embed=mcp` in the query, read in
  `Grooph::new` before the link clears it. Storage is neither read nor
  written (also not egui's memory); no update check, wake lock, MIDI,
  microphone, calibration, unsaved-changes dialog or keyboard shortcuts but
  Space. Inline, where the host offers fullscreen, `embed_ui` shows the
  compact card (`Grooph::compact_card`): a tempo row (`♩ = 140`, − and +,
  the fullscreen button), the staff with its meter in front
  (`time_signature_first`) and the ring with the play button in its middle
  (count-in beats on it while counting in, stop while playing); no menus or popovers, as Claude's guidelines ask. The view
  passes the display mode as `groophHost.displayMode`/`displayModes` and
  asks for fullscreen with `ui/request-display-mode`. In fullscreen, or on
  a host without it, it is the Metronome tab without the tab bar (meter,
  tempo with − and + in the bar, play in the ring's middle, as in the app);
  the options sheet has "Open in grooph" instead of share and no "Edit clicks". The theme follows
  the host (`groophHost.theme`, the view calls `groophHost.repaint`), and so
  do its colors: the view resolves the host's style variables
  (`hostContext.styles.variables`, may use `light-dark()`) to
  `groophHost.colors`, which `embed::HostColors` applies to egui's panels,
  cards and text each frame (`apply_style`). Accents stay grooph's; without
  host variables egui's own colors stay.
- Loading: while the WASM loads (7.3 MB, about 3 MB compressed on the
  wire), `#loading_text` holds a skeleton of
  the card it becomes (tempo row with the tool's tempo, staff with its
  meter, ring with the play button; the bar below instead in the whole
  view), in the host's colors. eframe removes the element once the app runs.
- Safe area: the view passes `hostContext.safeAreaInsets` (what the host
  covers, e.g. Claude's composer over the bottom in fullscreen) as
  `groophHost.insets`; `embed::host_safe_area` hands them to egui in
  `raw_input_hook`. `Grooph::ui` lays the app out inside
  `ctx.content_rect()` for every platform, so the Android wrapper only sets
  its system bars' insets. Claude sends 12 px
  inline (the card's padding) and, in fullscreen, 88 px at the bottom for
  its composer.
- The metronome's staff (in the app and embedded) takes its size from the
  width alone, and its panel only the height it needs: `staff_and_ring`
  works it out in the same frame (`metronome_staff_height`, from
  `staff_opts` and the panel frames' margins) before it sizes the ring
  below with the rest, so a resize shows at once.
- Accessibility: eframe gives the web no accessibility tree (AccessKit is
  native only), so a screen reader sees an empty canvas. The view adds a
  small DOM layer: a `role="status"` live region with the model context's
  text, a play/stop button (`groophHost.togglePlay`, defined by the app) and
  a label on eframe's hidden text field. Text colors meet WCAG AA app-wide
  (`style::readable_text`, egui's weak and dark text are too faint). The
  settings go to the model through `groophHost.updateModelContext` once they
  stayed the same for a second. The bridge is in `web.rs`/`platform.rs`.
  Inline, touch and wheel events are left to the page (`web_options`,
  `embed::chat_takes`; the canvas has `touch-action: pan-y`), so a vertical
  drag on the view scrolls the chat, as Claude's guidelines ask; taps and
  horizontal drags stay with the app. In fullscreen no chat scrolls behind,
  and the guidelines leave vertical scrolling to the app: the canvas gets
  `touch-action: none`, eframe prevents the defaults again, and pickers
  scroll and the options sheet folds on a flick as in the app. The view moves with the page
  while it scrolls, so egui would take the scroll for a tap where it began:
  on `touchcancel`/`pointercancel` `embed::cancel_scrolled_touch` moves the
  pointer far off before releasing it. The options sheet does not fold on a
  flick there.
- After every deploy CI runs `node mcp/smoke.mjs <url>/mcp` against the
  deployment: the MCP Inspector's CLI (pinned), the official client, lists
  the tool, calls it once with settings that open and once with a misfit
  `poly` (`isError`), and reads the view's resource.
- Tests: `node --test mcp/server.test.mjs` (no Cloudflare needed; build the
  link check first with `cargo build --release --target
  wasm32-unknown-unknown -p grooph-link-check`), `cargo test` (embed
  mode). `node mcp/local-host.mjs [dist]` plays host and sandbox locally
  for a `trunk build` (spec CSP, `?wasm=1` adds `'wasm-unsafe-eval'`). To
  run the function as Cloudflare bundles it, start `npx wrangler pages dev
  dist` and the host with `GROOPH_ORIGIN=http://localhost:8788`.
- To try a branch in Claude: Settings → Connectors → Add custom connector
  with the preview's `https://<branch>.grooph.pages.dev/mcp`.

## Guide pages (crates/site)

- Markdown under `crates/site/content/` becomes static pages on grooph.app
  (`content/learn/x.md` -> `/learn/x/`). Front matter: `title`,
  `description`, `section` (Metronome, Sight-reading, Learn, Features),
  optional `heading` and `order`.
- Fenced blocks: ` ```rhythm bpm=90 swing=66 | Caption ` (text notation,
  rendered as a grid with counting syllables from the app's counting
  overlay (`CountingSystem::Numbers`) and a play link), ` ```open |
  Label ` (link query, rendered as a button), ` ```pages ` (list of a
  section). `## FAQ` with `### Question` headings becomes FAQPage data.
- Runs as a Trunk `post_build` hook (see `Trunk.toml`), so `trunk build`
  writes the pages, `site.css`, `sitemap.xml` and `llms.txt` (base file plus
  a list of guides) into `dist/`. By hand: `cargo run -p grooph-site -- dist`.
- `cargo test -p grooph-site` builds all content and checks every rhythm and
  every link (app links through `grooph_link::parse_query`).

## Generator / endless mode

- UI: the Practice tab of the layout (`crates/app/src/shell.rs`). The
  settings card opens a picker per setting (`crates/app/src/pickers.rs`);
  changing a setting regenerates Practice's rhythm (undoable; never the
  score tab's score, see "Rhythm documents"), 🎲 rolls a new rhythm
  with the same settings. Space and the grouping picker live in the Level
  picker. There are no presets of the generator settings (they were
  removed); stored ones are dropped on restore, see "Saved state".
- App glue: `crates/app/src/generator.rs`. Endless mode (Bars: Endless; on the practice page, not in its editor)
  starts with 2 fresh bars on every play (no 🎲 then) and never loops: once the cursor reaches the last measure
  a new one is added below it, and the first measure is removed once its row
  has scrolled out of view and the audio cursor has left it. Removing it moves
  everything back by its ticks: the audio cursor (`Audio::drop_leading_ticks`,
  applied together with the shorter score), the playback clock, the accuracy
  marks and the scroll tween (by one row), so nothing jumps. A tempo ramp
  counted in loops counts in bars there (`played_ramp`).
- Measure widths (`staff_layout::min_measure_width`) are sized for at least a
  measure full of sixteenths, so up to that density they depend only on the
  meter: editing or adding measures (endless mode) never reflows the staff.
  Denser measures (32nds, big tuplets) grow with their beat count.
- Groupings: the one-beat figures are not a hand-made list but every onset
  pattern of a beat's grid (`all_patterns`: sixteenths over a quarter, dotted
  quarter, eighth or dotted eighth, and eighth triplets over a quarter). A
  `FigureId` is the pattern's position in that enumeration, `FigureSet` a
  128-bit mask over them. A pattern's level comes from rules (start on
  "e"/"a", or the count of dotted or syncopated notes and later starts: none 3, one 4, more 5) and the reference app's triplet
  ladder for three-slot patterns. A complexity level is a preset subset
  (`complexity_figures`): half of the beats draw from its highest level, the
  rest from one of the easier levels (each equally likely), then a figure of
  the level uniformly; `custom_patterns` (switched by
  `custom_figures_enabled`) in `GeneratorSettings` replaces it with a
  hand-picked subset (drawn uniformly; an empty selection falls back to
  complexity). Picker UI: "Groupings" in the generator panel, one row per
  level, tiles via `tool_palette::notation_button`.
- Ghost notes (`grooph_measure::ghost::ghost_onsets`): quiet hits on every free
  slot of the generator subdivision (per beat: 8ths=2, 16ths=4, triplets=3;
  mixed = triplets in tuplet beats, else 16ths). Other beat units get a straight
  8th or 16th grid (triplets → 8ths, mixed → 16ths). Scheduled as `SoundType::Ghost`,
  volume via the mixer's "Ghost notes" slider (`AudioSettings::ghost`).
- Count-in (`PlaybackOptions::count_in`): `TickSource` plays one bar of
  clicks on every beat unit of the first measure (of the loop range, if any) (`SoundType::CountIn` on the
  downbeat and primary beats, the quieter `CountInSoft` in between, so 7/8
  counts in all seven eighths) before the score; `Audio::playback_position` is negative meanwhile. The UI
  holds the cursor at 0 (`CountInState`) and re-anchors the accuracy session
  when the count-in ends.

## Sounds

- Three voices (`crates/audio/src/sounds.rs`): the rhythm (`Beat`, `AccentedBeat`,
  `Ghost`) uses `AudioSettings::rhythm_sound`, the metronome (`Downbeat`,
  `PrimaryBeat`, `SoftBeat`, `CountIn`, `CountInSoft`) `metronome_sound`, the
  counter rhythm (`Counter*`) `counter_sound`. All three are a `Sound` (Click,
  Claves, Snare, Xylophone); defaults are `Sound::RHYTHM_DEFAULT` (snare),
  `METRONOME_DEFAULT` (claves) and `COUNTER_DEFAULT` (xylophone).
  `SoundType::patch` turns a hit into a `Patch`: the configurable `Click`
  (waveform, frequency, noise) or an embedded WAV sample, both with the mixer
  gain of that sound type.
- Every `SoundType` has a `Level` (strong: downbeat, accents, strong counter
  clicks; normal: primary beats, notes, count-in, medium counter clicks; soft:
  soft beats, ghost notes, soft count-in and counter clicks). A sample sound's
  `SampleSet` has one recording per level; without a soft one, soft hits play
  the normal recording, quieter through the mixer. Claves: `claves3.wav`
  (strong) and `claves4.wav`; xylophone: `xylo-high.wav` and `xylo-low.wav`;
  snare: `snare-accent.wav`, `snare-normal.wav` and `snare-ghost.wav`.
- A head of a drum kit instrument plays the kit (`SoundType::Kit`, a
  `KitHit` of instrument, technique and level), whatever the voices' sounds;
  an `Instrument::Rhythm` head stays the rhythm's `Beat`/`AccentedBeat`, so a
  one-line score sounds as before. `kit_sample` picks one sample per
  instrument and technique: the snare plays its three recordings by level
  (cross-stick and rimshot their own), the hi-hat closed, half-open or open,
  the ride its bell, the crash choked; the toms play one sample, a rimshot as
  their normal hit. The other levels play quieter or louder
  (`KitHit::level_gain`). The kit samples are in `crates/audio/samples/kit/`,
  synthesized by its `synth.py` (standard library only, seeded); rerun it after
  changing a sound. An open hi-hat is not cut off by the next closed one.
- The rhythm and the counter rhythm play a sample 1.5× faster
  (`SAME_SOUND_RATE`) when the metronome uses the same sound, so the voices
  stay apart.
- Samples live in `crates/audio/samples/` and are listed in `SAMPLES`
  (`include_bytes!`, decoded once by the small decoder in `wav.rs`).
  Samples are stored as 16-bit mono WAV, trimmed after the decay, to keep the
  WASM build small.
  Samples keep their own license (claves: altemark on freesound.org, CC BY-NC; xylophone: Brsjak on
  freesound.org, CC BY 4.0),
  see `crates/audio/samples/CREDITS.md`; add a row there with every new sample.
- `VoiceMixer` (`voices.rs`) plays samples resampled to the device rate.
- Mixer (`mixer_panel.rs`, a picker on the More screen): a "Rhythm" group (`beat` as "Notes", `accent`,
  `ghost`), a "Kit" group (`AudioSettings::kit`, a `KitVolumes` of kick,
  snare, hi-hat, toms and cymbals) and a "Metronome" group (`downbeat`,
  `primary` as "Medium", `soft`, `counter` as "Polyrhythm", `count_in`), each
  with a mute (`rhythm_muted` / `kit_muted` / `metronome_muted`, applied in
  `SoundType::profile`). The metronome mute also
  silences the counter rhythm and leaves the count-in audible.
  Sounds with zero gain are not triggered at all. Clicks between the beats
  (e.g. every eighth of 7/8) come from the click bar, not from the mixer.
- Click bars (`grooph_measure::click`): what the metronome clicks in a meter
  is a measure, the meter's click bar. Each note is a click, its `ClickLevel`
  from the note: accented = strong, plain = medium, soft = soft, a rest = no
  click; tuplets and dotted notes work as in any rhythm. The default bar
  (`default_click_bar`) has one note per beat group, as long as the group,
  the first accented; `subdivided_click_bar` splits every beat unit into 1 to
  9 clicks (group starts keep their level, the rest are soft). `subdivisions`
  lists the subdivision picker's options (`none` for the default bar, then
  each split by its notes' code: `e`, `s`, `t16`, …). `ClickBars`
  (built from the metronome's document, see below) keeps the bars that differ from the default,
  keyed by the whole `TimeSignature` including its grouping. The metronome
  always plays them. Practice and the score tab have one "Click" row for
  both, each its own: its switch (`PlaybackSettings::click`) mutes the click
  (`Played::Rhythm`, no click bars; the count-in still clicks), its picker
  chooses "Standard", every meter's default bar (the default), or
  "Metronome", the edited bars (`PlaybackSettings::metronome_click`, see
  `Grooph::played_click_bars`), showing the bar each meter of the played
  score clicks under the one chosen. `Schedule::build` plays strong as `Downbeat`,
  medium as `PrimaryBeat`, soft as `SoftBeat` (mixer "Soft"), at the swung
  position of each note. The count-in and calibration
  ignore the click bars. Link parameter `click` is the click bar of the
  link's first meter in text notation without the time signature (e.g.
  `>q r:q. (q)`); shared links carry it when it is not the default and is
  played (from Practice or the score tab only with the "Metronome" click;
  a rhythm's link picks "Metronome" if it carries one, else "Standard"), and
  write a picker option as `subdivision=<code>` instead (`subdivision_of`);
  a link takes one of the two.

## Metronome trainer

- `grooph_audio::{GapClick, TempoRamp}` (`crates/audio/src/trainer.rs`), carried in
  `PlaybackOptions`. Both depend only on the bar count since the score started
  (`TickSource::bars`, across loops; with a loop range the bars played in it,
  a loop being one pass of it), so nothing rebuilds the schedule.
- Both are part of each page's `PlaybackSettings`: the metronome, Practice and
  the score tab each have their own (`settings::TabPlayback`, read through
  `Grooph::tab_playback`, the page shown or behind More), with tempo, count-in,
  swing and click, so a practice setting on one page stays there. Count
  labels, guide, timing feedback, sticking and the mixer are shared.
- Gap click ("Silent bars"): the audio thread decides per bar (`GapDecider`, random mode seeded via
  `Audio::set_seed`) and filters triggers (`SoundType::muted_by`). The count-in is
  never muted. `GapMute` says what an off bar silences: `Metronome` every click
  ("All clicks"), `Everything` clicks and notes, `ClickBar` the click bar's
  clicks alone and `Counter` the counter rhythm's alone ("Click bar",
  "Polyrhythm"). Practice and the score tab offer the first two, the metronome
  with a counter rhythm all clicks and the two layers, without one no choice
  (`Grooph::offered_mutes_of`); a value the page does not offer plays as all
  clicks there and stays saved (`played_mute`, which `playback_options`
  sends). The row's value names a layer first ("Polyrhythm · 4 on · 4 off",
  `silent_bars_label`). `Audio::bar_state` publishes bar + muted flag
  (`PlaybackState::bar_muted`, with the mute played in `bar_mute`). A bar
  silent as a whole has a veil and 🔇 on the staff; a bar silencing one layer
  draws that voice with faded ink (`RenderContext::faded`) and no veil
  (`measure_panel::StaffSilence`). The ring dims the silent layer's ring
  (`RingSilence`). A bar silent as a whole shows no playback position (no
  line, no lit note, no hand on the ring); while one layer sounds, the
  position and the hand go on. The
  flash follows the click bar (`PlaybackState::flash_shown`), and timing
  feedback counts any silent bar as silent. Silent bars are not in links and
  not undoable.
- Tempo ramp: `TempoRamp::bpm_for_bar` from the page's BPM setting; the audio thread
  scales ticks per sample, the UI (`metronome_trainer.rs`) uses the same tempo in
  `score_timing()` via `PlaybackState::ramp_bpm` and re-anchors the accuracy
  session on every change. Each page's options sheet has a "Tempo ramp" row
  next to "Silent bars" (`Picker::TempoRamp`); "Keep the reached tempo" sets
  the tempo of the page that played (`stop_transport`).

- Metronome tab (`crates/app/src/metronome_view.rs`): its own meter
  (persisted, picked in the tab's bar), separate from the generator's.
  The tab's document (`EditorState::docs.metronome`, see "Rhythm
  documents") is the one owner of every meter's click bar, counter rhythm
  and ratio name (`metronome_bars.rs`, one `MeterBars` per meter), open or
  not. Its score is the bars of the meter it clicks, and that bar's meter
  is the metronome's (`metronome_meter`), so the staff, tool palette and
  undo work on them through the `Doc` trait; `MetronomeDoc`'s private
  `MetronomeBars` holds that meter's ratio name and the other meters' bars.
  Both are in the undo snapshot, so a meter change, a click edit and a
  ratio pick undo as one. The score is private to `metronome_bars.rs`, so
  the compiler keeps meter changes to `MetronomeDoc::click_meter` (picker,
  link; undo restores the whole document), which keeps the old meter's
  bars, and `keep_meter_bars`, which sets a meter's bars without changing
  the meter (a link's click bar, a ratio pick). Every other edit goes
  through `Doc::edit_score`, which takes back (and reports, as `None`) one
  that would change the meter or add a bar. `click_bars`/`meter_bars` read them: the audio engine
  takes the click bars when it plays a rhythm, persistence saves them as
  one `SavedMetronome`. A metronome link sets the meter and clicks
  in place, also with the tab open. The audio engine
  plays the document itself as clicks (`Played::Clicks`): its first voice
  is the click bar, its second the counter rhythm, so only clicks sound and
  hits are measured against the same score; ghost notes, endless mode and play along are off there. Below the
  staff a ring shows one segment per note or rest of the bar, as long as it,
  thickness by level; a tap cycles strong, medium, soft, off (undoable). The
  "Clicks" picker offers the ready-made bars (`click_fills`), "Edit clicks"
  shows the tool palette. A meter picked there always starts over
  (`set_metronome_meter`, undoable): its click bar is `meter_click_bar`
  (beats in x/4, every eighth or sixteenth in x/8 and x/16, stored as the
  meter's click bar) and it has no counter rhythm, whatever it had when it
  was last picked. The metronome's meter and the rhythm's never change each
  other (Practice's meter picker, the editor's meter cell, which opens `Picker::BarMeter`, the metronome's meter grid on the cursor's bar). The staff gets a share of the free height
  (`METRONOME_STAFF_SHARE`), the ring the rest; the options sheet folds away
  like Practice's (shared `options_collapsed`); its bar holds the meter, the
  tempo with − and + and share. The ring's middle is the play button
  (count-in beats on it while counting in, stop while playing). While
  playing, the click and beat being heard light up. Switching into or out of the tab
  stops playback.

## Play along (accuracy)

- Hit sources (`crates/app/src/play_along.rs`): MIDI NoteOns, microphone hits
  and taps (tap key, default `J`, or a press on the staff while playing). All
  hits are stamped on one clock (`grooph_clock::now_seconds`, `crates/clock`); MIDI and
  microphone timestamps are moved onto it by their age. On the web taps use the DOM event `timeStamp` (`web.rs`), natively
  the frame time. Each source has its own input offset (`PlayAlongSettings`).
- While tap capture is active, staff presses (and presses on the metronome's
  ring) are hits, not edits.
- With a loop range only its notes are listed and judged
  (`compute_global_beat_onsets` over `timing.loop_bars()`, neighbours linked
  round from `b` to `a`); notes outside it are never missed. The session's
  ticks stay the score's, on a loop from `loop_start_tick` (`Lane::at_or_before`,
  `wrap_tick`), so marks keep their keys. A new range carries the run on
  (`AccuracyTracker::loop_changed`): the open stretch is made final with the
  hits it has, the lanes start again at the clock's new anchor, and counts,
  marks and extra hits stay until the run ends.
- Where (`Grooph::accuracy_active`): with Timing feedback on, in Practice, in
  the metronome tab (its click bar and counter rhythm) and in the score tab
  (its score; not while it is edited, `Mode::Edit`), never embedded in a
  chat. The switch is in each of these tabs' options sheet.
- One-line scores: every hit (each MIDI NoteOn of any note and channel, tap
  and microphone hit) counts for a note of any voice, with no catch window;
  each onset is one note. Which note is the alignment's (below).
- Kit scores (`StaffKind::Kit`): each head (an instrument at an onset) is one
  note, and hits, misses, extra hits, the score, drift, beat positions and
  the voice and bar tallies all count heads. Only MIDI plays a kit: the drum
  map (`crates/app/src/drum_map.rs`, a table note → instrument and
  technique: General MIDI plus the e-kit notes 22, 26 and 58, on any
  channel) gives each NoteOn its instrument. A MIDI device can have its own
  drum map (`CustomDrumMap`, saved in `DeviceSettings::drum_maps`), which
  then replaces the table whole; a device without one plays by the table.
  Maps are keyed by the port name (`drum_map::device_key`), under Linux
  without the ALSA ` client:port` numbers at its end, which change on
  replugging; two identical devices share one. They are learned in the drum map list
  (`crates/app/src/drum_map_list.rs`, Play along → MIDI input → Drum map:
  pick a row, hit the pad); while it is open, the device's NoteOns go to it
  alone, even with playback stopped, and opening it stops and discards a
  run. Notes the map does not list, the hi-hat pedal (by its mapped
  instrument, not its note) on a score without hi-hat foot notes, taps and
  microphone hits are ignored (a press on the staff stays a tap, the microphone stays
  closed). The technique is read but not graded. A hit is aligned only
  with its instrument's heads, within the catch window
  (`KIT_CATCH_WINDOW_MS`, 150 ms, at most the gap to the instrument's
  neighbouring note on the hit's side); a hit with none there is extra.
  `accuracy::session::global_beat_onsets` lists one entry per head there
  (one per onset on one line), each linked to its instrument's neighbours
  (`prev_same`, `next_same`), which give the narrowed windows and the
  clamp of the marks; marks are keyed by onset and instrument (`NoteKey`)
  and drawn next to each head.
- Extra hits (a hit the alignment pairs with no note; on a kit also a hit
  with no note of its instrument in the catch window) are kept by the
  recorder as
  `ExtraMark`s, on every page and staff: placed from the nearest note as a
  hit is from its note, and kept until the cursor passes their place in a
  later loop (after stop, until the next run or an edit). The staff draws
  them as a ring (`draw_extra_marks_for_measure`; on the kit at the
  instrument's line, on one line in the marks' row), the metronome's ring
  on the ring of the nearest click's voice. They change no count.
- Microphone (`crates/mic`): a hit starts where the first sample crosses the
  threshold; 40 ms after it are ignored. On the web an AudioWorklet forwards
  1024-frame blocks to the page, whose context time is mapped onto
  `performance.now()`; natively cpal's capture timestamp is used. The
  microphone is open only while a run with it plays, a microphone calibration
  runs or the Play along dialog shows its level meter, because phones change audio
  routing while recording. On iOS the built-in microphone is chosen over a
  headset's, which filters out hits.
- The session (`accuracy::AccuracyTracker`) runs while MIDI is connected or
  after the first tap or microphone hit of a run. Stopping keeps a `RunSummary` (hits, perfect/
  good/off windows, misses, extra hits, mean, spread, and the tight counts:
  the hits graded again around the run's mean; `TightTally` keeps the
  `GradedHit`s and regrades them on each new hit, since the mean moves, so
  the live status line can show them too) for the "Your run" window,
  a `Drift` (each hit's deviation at its note's time, misses, silent
  stretches; `accuracy/drift.rs`) for its chart (`drift_chart.rs`, drawn
  with the painter, no plot crate), `BeatPositions` (the hits and misses
  per position in the beat, `accuracy/positions.rs`; each onset's position
  comes from `counting::position_in_beat`, the coarsest count with a part
  there, every beat's start being one position; positions the counting
  system names the same are merged when the strip is drawn) for the strip under it,
  and the run's marks, drawn on the staff (and the metronome's ring) until
  the next run or an edit. Hits and misses are also tallied for each of the
  first two voices (`VoiceTally`; a note several voices play counts for
  each), which the metronome shows for its click bar and counter rhythm.
  With the gap click on, the same tallies are kept for the bars with the
  click and the silent bars (`BarTimeline`). Each judged note waits with its
  time until a frame well inside its bar reads `Audio::bar_state` (via
  `PlaybackState::bar_muted`); near a bar's edges the engine's bar and the
  session's clock may be a frame apart (`ScoreTiming::measure_span_at`
  gives the bar's span). Notes in a bar no frame saw are left out; those of
  the last bar seen are tallied when the run ends.
- Notes are judged behind the clock by the largest offset of the sources in
  use plus a delivery margin (`miss_delay_s`, at most a quarter of the loop),
  so a hit delayed by input latency is in before its note is judged. The
  progress cursor waits after anchoring the clock (`anchored_ahead`), and a
  hit's loop is the one nearest to that cursor.
- Grading (`accuracy::session::HitWindows`): perfect ≤ 25 ms, good ≤ 60 ms,
  narrowed to 1/5 and 2/5 of the gap to the closest neighbouring note (as
  heard; on a kit score, of the same instrument) for fast notes.
- Rests are no neighbours (`link_neighbours`): they are often only how a
  note's length is written, which a hit does not tell, so a note is judged
  the same as a quarter or as an eighth and an eighth rest.
- Hits and notes are paired by an alignment over a short sliding stretch
  (`accuracy/align.rs`), as score-following systems do, not each hit at
  once to its nearest note. Per instrument (one lane on one line, one per
  instrument on a kit, independent of each other), the notes follow each
  other on one continuous timeline across the loop's end (occurrence `n`
  is the lane's note `n mod len` in loop `n div len`). A note's window
  (`Lane::window`) reaches on each side up to the neighbouring note of its
  lane, on a kit at most the catch window (`catch_window_ticks`:
  `KIT_CATCH_WINDOW_MS` there, unbounded on one line), and a hit is a
  candidate for every note whose window holds it. On one line every hit thus
  lies in some note's window; on a kit a hit with no note of its
  instrument within 150 ms is extra at once. The recorder buffers each hit
  with its lane (`LaneState`); the open stretch's hits and notes are paired
  in order (no crossing pairs, `Lane::align`, a dynamic programme) to
  minimise the sum of the pairs' costs plus `ALIGN_COST` (¾) for every
  pair fewer: only the sum of a miss's and an extra's cost matters. A
  pair's cost is its deviation in shares of the gap from the note to its
  neighbour on the hit's side, so the rule works alike at every tempo: a
  late hit on a note whose neighbour gets its own hit counts for it while
  it is less than ¾ of the gap late.
- A note and what it was paired with become final once the progress
  cursor reaches the midpoint between the lane's next note and the one
  after it, at most `LOOKAHEAD_CAP_MS` (500 ms) after the next
  (`Lane::final_at`); the hits before its hit, and those no open note's
  window holds any more, are then extra. Only final results are recorded
  (`recorder::finalise`, from `update_progress`): marks, extra rings, the
  counters, the tight tally, drift, beat positions, the voice and bar
  tallies. A mark thus shows up to about 1.5 gaps after its note (at slow
  tempos a gap plus 0.5 s), and nothing changes once shown. When the run
  stops, the open stretch is aligned with the hits in and the summary
  includes it; a note whose window is still open then and has no hit is no
  miss. A lane runs in the order its notes sound (swing can move a note
  past one of another voice); its first open note is kept by its written
  onset (`NoteAt`), unique in the lane and kept when swing changes. A miss
  is dated by when the cursor passed its note (`CursorLog`), at the tempo
  that played then. It starts at its note at or before the
  session's origin (the cursor at the start, or where marks were cleared):
  a first hit a little late counts for it, but a note before the origin is
  never missed. Re-anchoring the clock keeps the open stretch, whose notes
  and hits keep their ticks; a hit keeps the tempo it was played at, so a
  tempo change before its note is final does not change its deviation.
  Before Endless drops a bar, its open notes are made final with the hits
  in (`Until::Before`).
- The calibration's measure stays the nearest note: `record_hit` returns
  each hit's deviation from it at once (on a kit within the catch window),
  whatever the alignment later makes of the hit.
- Marks are clamped to the whole gap to their neighbours, as written
  (`accuracy::session::neighbour_gaps`; on a kit of the instrument): a hit
  can count for a note up to its neighbour. Markers are coloured by grade,
  and the connector shows which note a mark belongs to.
- A mark is drawn where the hit landed on the score's ticks round the loop
  (`measure_panel::staff_point`): an early hit on a bar's first note in the
  bar before, one on the score's first note at the end of its last bar, a
  late one past a bar's end in the next. Its connector then leaves the
  note's bar at its edge and comes into the hit's bar from the other one
  (`hit_path`). `MarkPainter` draws the marks of both staffs, `MarkRow`
  gives their height in any bar from the line the layout puts heads on.
- Calibration (`crates/app/src/calibration.rs`): plays a click bar of its own
  (4 quarters at 100 BPM, counted in, no swing, ghost notes, gap or ramp)
  and takes the median of the hits after a warm-up as the source's offset.
  It touches no page's document, tempo or option and leaves the page shown:
  while it runs, `played_score`, `played_bpm` and `playback_options` (and
  so `score_timing`, the audio, the accuracy tracker and the staff) give its
  bar instead of the page's. It is started from More → Play along
  (`play_along_settings.rs`), whose dialog stays open underneath.

## Rhythm documents

- Three documents (`crates/app/src/documents.rs`), one per page: the
  metronome's click bar, Practice's generated rhythm (thrown away freely)
  and the score tab's score (written, loaded from the library or opened
  from a score link), on the kit's staff or the one line (#352): the
  library's "New drum groove" and "New rhythm" start one
  (`Grooph::new_score`; `kit_palette::empty_groove` is also the score tab's
  score of a fresh state, `PersistedState::default`, while a saved score
  whose field does not read falls back to the one line's bar, which the
  score tab shows as it is). Every library pattern opens there
  (`load_pattern`), on either staff, also while the score tab is hidden
  (it then shows in the tab bar while open); nothing loads into Practice.
  Practice saves its rhythm as a new pattern ("Save rhythm 💾" in its
  options sheet, `request_save_practice_rhythm`; the score tab's unsaved
  changes ask first, `PendingLoad::PracticeRhythm`): the library opens on
  its name over Practice, and once it closes the pattern opens in the score
  tab (`LibraryUi::open_on_close`, `open_practice_save`, run every frame),
  unless it was deleted or another score replaced
  the score tab's meanwhile (a pattern, a new score, a link:
  `replace_score` drops it). Practice's document keeps its rhythm and no pattern. A score
  link of the one line opens on Practice unless shared from the score tab
  (`opens_on_practice`). The library keeps the score tab's tempo
  (`PlaybackSettings` of the score tab, whichever page saves), loading sets
  it, and "Save rhythm" stores Practice's. Only a tempo change in the score
  tab marks its score unsaved (`ScoreChange::Tempo`). A link's tempo, swing
  and click go to the page it opens (a tempo-only link: the page shown), and
  a written link carries the open page's values; the embed and the MCP tool
  report the metronome's.
  Each keeps its score, cursor and undo history (the `Doc` trait, all
  three in `EditorState::docs`); Practice's and the score tab's
  (`RhythmDoc`) also their dirty flag and pattern, the metronome's
  (`MetronomeDoc`) every meter's bars. `EditorState::doc`/`doc_mut` give
  the shown one as `dyn Doc`, whose score changes only through
  `edit_score`; `rhythm_doc_mut` gives it unless it is the metronome's.
- Each page edits its own document in place: the metronome's "Edit clicks",
  Practice's ✏ ("Edit rhythm") and the score tab's ✏ (`edit_page`) open
  `Mode::Edit` on the page shown, Done, Back and Esc close it. Nothing is
  swapped: the page shown, or the one the More screen was opened from, is
  `EditorState::document`, and the staff, tools and undo work on its
  document (`EditorState::doc`/`doc_mut`). `show_document` changes it and
  stops playback; `select_tab` shows the tab's document.
- The metronome's document holds its click bar and counter rhythm whether
  the tab is open or not; nothing copies them back and forth.
- The generator writes Practice's document only: `generate_new_score` shows
  it first, so no generator action (🎲, a setting, a generator
  link, endless mode) replaces the score tab's score. Endless mode and ghost
  notes run on Practice's rhythm only.
- Each rhythm document has its own loop range (`RhythmDoc::range`; see
  "Audio, playback, counting"). Setting, moving or switching it off
  (`RhythmDoc::set_range`) is no undo step and nothing unsaved. The undo
  snapshot (`EditorSnapshot::range`) keeps it with the score and how often the
  user had changed it then (`range_changes`): undo and redo bring it back with
  the bars it covered, unless the user changed it since, which then stays,
  cut to the score (one switched off stays off). Duplicate bar, Delete bar and
  the ghost bar move it with its bars (`BarRange::after_insert`/`after_remove`;
  a copy of `b` and an appended bar lie outside, the only bar deleted clears
  it). `replace_score` drops it (`drop_range`), so another score opens without
  one and undo brings the old one back with it; switching endless mode on
  deletes Practice's (`switch_endless_on`).
- The library and the unsaved-changes checks reach the score tab's document
  as `docs.score`, from any page and without closing the metronome. A tempo
  change marks it unsaved on the score tab, and on any page when it is a
  saved pattern (`ScoreChange::Tempo`, see Change navigation).
- Persistence saves the rhythm documents and the page: each document as
  one `SavedDoc` (score, cursor, `dirty`, `active_pattern_id`), the score
  tab's as `score_doc`, Practice's as `practice_doc`, and `document` the
  page behind More. `SavedDoc::of`/`into_doc` turn a document into its
  entry and back, the same way for both. Within an entry, too, a field that
  does not read falls back alone (`persist::repair_doc`). The loop range is
  saved in the entry (`range`, written only when set; one that does not fit
  the score, or ends before it starts, reads as none), never in the library or
  links. A version 2 state
  kept the score tab's document in flat fields and Practice's rhythm as
  `practice_score`; `persist::migrate` moves them into the entries without
  a backup. A state saved before the two documents has no Practice
  rhythm: its score stays in the score tab and Practice starts with a new
  one. `Tab::Score` still reads `Edit`.

## Layout

- One layout for every window size, made for touch first
  (`crates/app/src/shell.rs`). On windows wider than 720 pt the controls stay
  a centered column; the staff and the tool palette use the full width.
- Tabs: Metronome (see below), Practice (generator card with
  Meter/Notes/Level/Bars/BPM, options sheet, big "new rhythm" + play
  buttons; ✏ in its options opens its editor), Score (its own score, see
  "Rhythm documents": a card with the score's name (a tap opens the library)
  and BPM, the options sheet without the generator's rows and ✏ / play /
  share), More (`Mode::MoreMenu`, a screen of its own without the staff: a
  list at the bottom whose rows open pickers, Theme is picked in its row).
  Each of Metronome, Practice and Score is in `Mode::Playback` (its card) or
  `Mode::Edit` (tool palette above an edit bar at the bottom with Done as
  ✔, and no tab bar: Done,
  Esc or Back leave the editor, `leave_editor`, and the page itself shows:
  `shell_ui` folds the options sheet as the editor opens, its animation
  snapped shut, so it does not slide shut after Done) on its own document; Esc
  switches between the two, Back closes the editor. The tab is derived from
  the document (`tab`), More from `Mode::MoreMenu`. More stays in front of
  an open metronome and leaves it playing (`advance_unseen_playback` moves
  the playback on without a staff); the library has no rhythm to save
  there, and a loaded pattern opens on the score tab.
- The library (`library_panel.rs`) holds the saved rhythms as cards
  (`library_card`), from every page. They are the score tab's: a tap loads
  one into it, ✏ renames or deletes it, and its marks (active, unsaved) are its pattern's. The save
  buttons are those of the page the library was opened from
  (`rhythm_saving`, by `EditorState::document`): from the score tab "Save"
  and "Save as new" keep its score (`docs.score`), from Practice "Save as
  new" saves Practice's rhythm as its "Save rhythm 💾" does, from the
  metronome there are none.
- Settings open in a `Modal` picker (`settings_picker`); generator changes go
  through `finish_generator_change`.
- The staff's em is capped so one row fits the panel's height. The row's
  height is measured from its laid-out notes (`StaffLayout::room`,
  `RowRoom::height`).
- Windows narrower than `NARROW_MAX_WIDTH` (600 pt, `StaffView::narrow_screen`)
  get tighter staff spacing, so glyphs are bigger on phones.
- While a tempo ramp runs, the card shows the tempo being played ("BPM now").
- Share (right of play, past the loop button; not in the editors) builds a score link with tempo
  and swing; on the web it opens the system share sheet
  (`platform::open_share_sheet`, Web Share API), else copies the link. In the
  Metronome tab, share (right in the bar) builds a `ts` link with tempo, swing
  and the click bar instead.
- Keyboard shortcuts are ignored while a text field has the focus.


- Tool registry: `crates/app/src/tools.rs` (ToolKind, Modifier, shortcuts).
- Keyboard handling: `crates/app/src/keyboard_input.rs`.
- Measure operations: `grooph_measure::editing::{Modification, set_tuplet, ...}`.

The tool palette (`tool_palette_panel`) is one grid for every editor
(Practice, Score, the metronome's click bar): rows of cells
(`tool_palette::grid_rows`), on the kit its instruments, each with the techniques of its head on the beat, then
one row of the note values, the rests, the head modifiers (accent, ghost, dot), the stickings (R, L and both,
`Modifier::ToggleSticking`, greyed out on a beat without a hand instrument, not in the metronome), the tuplets and
the meter, each
row going on in the next line only where it does not fit. Undo and redo
(`BAR_TOOLS`) are in the editor's bar (`editor_bar`), right after play, Done
on the right. In Practice and Score the staff shows a dashed, empty "ghost
bar" after the last bar (`measure_panel`: laid out with the score, never
stored, and never reached by the playback cursor: `draw_staff` and
`current_cursor_x` take the bars that play, `loop_bars`); a tap on it appends a bar like the last one (`Grooph::push_measure`)
and puts the cursor there. The bar menu (`BarMenu`), from a "⋯" in the top
right corner of the cursor's bar or a long press (right click) on a bar,
cleans up (every voice, greyed out where nothing would change; see
"Clean up"), duplicates, clears (every voice) or deletes it; in the metronome it only
clears the voice being edited. A modifier's cell is greyed out where it
would change nothing on the cursor's beat (`modifier_applies`, tried on a
copy of the bar): an accent on a rest, a dot without room, a technique
without the head or its instrument's. Accent, ghost and dot show the cursor's beat as they
would change it (`modifier_preview`): its value with the mark. In a tuplet the
value buttons show the value they write there, the group's ratio, with its
number over it (`value_in_tuplet`, see `VoiceBar::modify_beat`), and a value
button is greyed out where its value does not fit from the cursor on
(`value_applies`). The tuplet the cursor's beat is
in shows as picked (`cell_selected`); a tap on it dissolves the group. A voice is picked by a tap on
it, or on the kit by tapping an instrument. The
columns follow the width of the page's column (`grid_metrics`, cells 54 to
64 px wide; grid and bar both in `centered_column`, so on a wide screen they
line up with the cards), a row
wider than the grid goes on in the next line, and past three and a half
lines or 45% of the screen's height the grid scrolls, so the staff stays in
view (the half line cut off shows there is more).

The whole note (`w`) and whole rest (`r:w`) are on the grid and in the palette, but only entered by hand: Clean up, the
generator and the fill (`best_fill_for_gap`) never write them. A whole note has the wide whole head and no stem; a
whole rest value is valid only in a bar of four quarters (`whole_rest_fits`), the empty bar's whole rest in any
meter is the layout's.

If you add new tools/shortcuts, remember:
- Tool registry + palette (`grid_rows`)
- Keyboard input
- UI help text

## Build and test

- Native: `cargo run --bin grooph-app`
- Tests: `cargo test`
- WASM (optional): `trunk serve` or `trunk build --release`
- Android: `crates/android/build-android.sh` builds `target/android/grooph.aab`
  (the App Bundle for Google Play) and `target/android/grooph.apk` (the same
  app to install directly), each for arm64 phones and the x86_64 emulator.
  It needs `cargo install cargo-ndk`, the `aarch64-linux-android` and
  `x86_64-linux-android` Rust targets, `ANDROID_HOME` (with
  `platforms;android-36`), `ANDROID_NDK_HOME`, Java and bundletool; the
  script lists its variables (keystore, version code). The manifest is
  `crates/android/AndroidManifest.xml`, the launcher icon (an adaptive icon
  with a themed layer) is in `crates/android/res`, the 512 px Play Store
  icon in `crates/android/store`. CI builds both on every push (job
  `android`, artifact `grooph-android`).
  - Signing: CI signs with the upload key when the repository has the
    secrets `ANDROID_UPLOAD_KEYSTORE_BASE64` (the keystore, `base64 -w0`),
    `ANDROID_UPLOAD_KEYSTORE_PASSWORD` and `ANDROID_UPLOAD_KEY_ALIAS` (the
    key's password must match the keystore's), else with a throwaway key.
    Create the key once with
    `keytool -genkeypair -keystore upload.keystore -alias upload -keyalg RSA -keysize 2048 -validity 10000`
    and keep a copy outside GitHub: Play only accepts uploads signed with it.
  - The workspace eframe leaves out accesskit (NativeActivity cannot use it);
    `crates/app` adds it back on every other target. Settings are stored in
    the app's internal storage (`persistence_path`). Android renders with
    glow (OpenGL ES): wgpu cannot create a surface in the emulator.
  - Calls into Java go through JNI in `crates/android/src/activity.rs`:
    NativeActivity draws under the status and navigation bars, so
    `insets.rs` reads their size and hands it to egui as safe-area insets;
    the share sheet and the microphone permission reach the app through
    `grooph_app::Host`. The back button arrives as `Key::BrowserBack`
    (the manifest opts out of Android 16's predictive back, which would
    stop sending it) and goes to `Grooph::back`, which closes what is open;
    with nothing open the app moves to the background. eframe's save when
    the window goes away is the only sign of going to the background, so
    the save checks whether the window is hidden and calls `Grooph::hidden`
    (stops playback, audio and microphone). NativeActivity's own request
    for the on-screen keyboard does nothing on current Android versions, so
    the wrapper opens it through the InputMethodManager while egui wants
    text input (logged as `Keyboard open`) and follows the insets (which
    include the keyboard) closely while it slides. Not handled yet: opening
    shared links in the app (Android intents).

## Test notes (measure)

- Tests live in the relevant modules (e.g. `crates/measure/src/...`).
- Helpers for duration in `crates/measure/src/duration.rs`:
  `q()`, `e()`, `s()`, `th()`, `t8()`, `t16()`, `t32()`, `qt16()`.
- Recommended: `Measure::set_beat` + `Beat::note/rest(...)` for targeted changes.
- Low-level checks: `DEFAULT_GRID.ticks_of(...)` and `compute_onset_ticks(...)`.
- Tuplet tests: note auto-fill and the groups (`tuplet_groups()`, or `==` on the measure).

## Change navigation (quick)

- App shell, tabs, options sheets, More menu: `crates/app/src/shell.rs`; dialogs: `pickers.rs`.
- App state and the frame loop: `crates/app/src/lib.rs` (`Grooph`), `state.rs` (its parts), `documents.rs`
  (the three pages' documents), `undo.rs`.
- Saved state: `crates/app/src/lib.rs` (`PersistedState`), `persist.rs` (field-wise restore, migration, backup).
  The user's presets of the generator settings, saved until they were removed (`presets`), are dropped on
  restore without a version bump; a state with any is backed up once (`persist::take_presets`), so they are not
  lost without a trace.
- Settings: `crates/app/src/settings.rs`, one `serde` group per feature (`PlaybackSettings`, one per page, `GeneratorConfig`,
  `DeviceSettings`, `FeedbackSettings`, `ViewSettings`; the sounds are `grooph_audio::AudioSettings`). The same
  struct is held at runtime and saved in `PersistedState`, so `from_app` and the restore copy groups. A new setting is a field in its group
  (with `#[serde(default)]` from the group, it falls back alone, see `persist::repair_field`), or a new group:
  then it gets an entry in `PersistedState` and `repair_field`. Renaming or moving a saved field bumps
  `PERSISTED_STATE_VERSION` with a step in `persist::migrate` (version 4 moved the flat fields of version 3 into
  the groups, `SETTINGS_GROUPS`; version 5 joined the metronome's `click_bars`, `counter_bars`,
  `counter_names` and `metronome_meter` into `metronome`; version 6 copied the shared `playback` group into
  each page's `metronome_playback`, `practice_playback` and `score_playback`, `persist::split_playback`). The default tempo is `grooph_measure::tempo::DEFAULT_BPM`.
- UI state: `UiShell` (`state.rs`) holds `mode`, `platform` and feature groups only (`Fonts`, `LayoutSettings`,
  `ViewSettings`, `StaffView`, `LibraryUi`, `Dialogs`, `ShellState`, the host and the embed). No new field goes
  directly into `UiShell` or `PlaybackState`: it goes into its feature's group, or a new group.
- Editing: `crates/app/src/tool_palette.rs` (grid, applying a tool), `tools.rs` (tool registry, shortcuts),
  `kit_palette.rs` (the kit's instruments).
- After a score or timing change: `crates/app/src/score_change.rs`. Call sites make the change, then report its
  kind to `score_changed` (`ScoreChange`: an edit or undo/redo, a replaced score and its `Origin`, Endless adding or
  dropping a bar, the tempo, the played timing, another document). It alone brings the accuracy session, the
  playback state, the cursor and `dirty`/`active_pattern_id` in line; `with_undo_snapshot`, `undo`/`redo` and
  `replace_score` call it. A new kind of change gets a variant there. The audio plan, the staff layout and the
  onsets are pulled every frame and need no report.
- Staff on screen: `crates/app/src/measure_panel.rs`; the metronome's ring: `metronome_view.rs`. `measure_panel`
  runs its steps in order, each a function of the parts it needs: `panel_frame` (playback moves on, the guide's
  anchor; a `PanelFrame`), `ShownScore::of` (the played score, the ghost bar) and `fit_to_height` (layout and
  shrink-to-fit), `follow_playback`/`playing_row_offset` and `follow_edit_cursor`/`edit_cursor_target`
  (auto-scroll), `hover_with_ring`, `draw_staff_view` (staff, flash, ghost bar), `bar_menu` (returns a
  `BarAction`), `draw_playing_marks` (muted bar, accuracy marks), then `staff_input` and `apply_bar_action`. Room
  for the staff goes into `staff_opts` or `fit_to_height`; a new overlay into `draw_staff_view` (under the bar menu)
  or `draw_playing_marks` (over it); new input into `staff_input`, or a new `BarAction` the panel applies last,
  once nothing borrows the score. The decisions (`fit_to_height`, `playing_row_offset`, `edit_cursor_target`) are
  free functions with unit tests; keep egui out of them.
- Playback cursor and count-in: `crates/app/src/playback_clock.rs`; play along: `play_along.rs` and
  `accuracy/` (`session.rs` onsets and grades, `recorder.rs` hits, `progress.rs` misses).
- Model/rules: `crates/measure/src/lib.rs`, `voice.rs`, `editing.rs`, `fill.rs`, `notation.rs` (text notation),
  `tempo.rs` (`ScoreTiming`), `counting.rs`, `click.rs`, `counter.rs`.
- Layout/geometry: `crates/layout/src/staff_layout.rs` (a score in rows), `tick_map.rs` (tick to x), `pixel_layout/` (one measure:
  `mod.rs`, `notes.rs`, `beams.rs`, `tuplets.rs`), `staff.rs` (lines, instrument places), `render_plan.rs`.
- Drawing: `crates/render/src/measure.rs`, `staff.rs`, `beat.rs`.
- Audio: `crates/audio/src/lib.rs` (engine, plan swap), `schedule.rs`, `tick_source.rs`, `sounds.rs`, `output.rs`.
- Links: `crates/link/src/lib.rs`; MCP server: `mcp/`, `functions/mcp.js`.
- MIDI: `crates/midi/src/input.rs`. Microphone: `crates/mic/src/` (detector, `native.rs`, `web.rs`).
- Platform: `crates/app/src/platform.rs` (web/native twins), `web.rs` (browser glue), `host.rs` (Android).

## Web runtime assumptions

The web build runs on the browser's one thread, and some code relies on that:

- `crates/app/src/web.rs` and `update_check.rs` keep browser state in `thread_local!`s (visibility changes,
  raw key presses, a cancelled pointer, the embed's play and stop requests, the build found on the server),
  written by DOM callbacks and read by the frame, which never run at the same time.
- DOM callbacks registered for the page's lifetime are leaked on purpose (`Closure::forget`): the page
  never removes them, and a dropped closure would throw when the browser calls it.
- `crates/app/src/embed.rs` keeps the embed's fullscreen flag in a `static AtomicBool` (`FULLSCREEN`), set
  from a host message and read in the frame; `Relaxed` ordering is enough on one thread.
- `crates/measure/src/click.rs` and `counter.rs` cache bars per meter in `thread_local!`s; each thread (a
  test, natively) has its own.

The audio thread is the exception natively: it shares only `PlaybackState` behind a mutex with the UI
(see "Audio, playback, counting").

## Notes for LLM agents

- Model changes can affect layout, audio, and rendering.
  Search for `DEFAULT_GRID` usage and tick logic.
- Web-specific logic is gated by `cfg(target_arch = "wasm32")`.
- Keep measure validation in mind: `set_beat`/`set_tuplet` must not leave
  unfillable measure lengths.
