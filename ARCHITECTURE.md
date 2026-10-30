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
- `midi` and `mic` stand alone: they deliver input events and know nothing about the model.
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
  A `Measure` reads and edits as its first voice (`Deref` to `VoiceBar`), so
  single-voice code (click bars, generator, counting, notation, audio)
  needs no change; `voices()`, `voice_mut(i)` and `push_voices` reach the
  rest. Measures saved before voices (beats and tuplets inline) load as one
  voice (`RawMeasure`).
- `Beat` is a slot of the rhythm, `duration` and `tuplet_group_id`, and what
  is played in it, its `Heads` (`grooph_measure::head`): none is a rest, one a
  note, several a chord on one stem (at most one per `Instrument`, sorted from
  the foot up, at most `MAX_HEADS`). Each `Head` has its `Instrument`,
  `accented`, `soft` (a ghost note, head in parentheses; never with
  `accented`: the setters clear the other flag, a save with both keeps the
  accent) and a `Technique` its instrument allows (open hi-hat,
  cross-stick, ...; checked when built or loaded). `kind()`, `accented()`,
  `soft()` read the beat as a whole (any head); `set_accented`/`set_soft` set
  every head, `set_kind` turns a rest into a plain note and back.
  Today every note is one `Instrument::Rhythm` head, the one line of the
  staff; `Instrument::staff_pos`/`notehead` hold the drumkit legend for kit
  scores (#251-#255). Beats saved before heads (`kind`, `accented`, `soft`)
  load as one `Rhythm` head (`RawBeat`).
- `Duration` is `Simple`, `Dotted`, or `Tuplet(TupletSpec { n, m, base })`.
- `Measure::set_beat` guarantees a valid measure length and fills gaps.
- `DEFAULT_GRID` defines the valid duration grid and tick calculations.
- Tuplets:
  - A tuplet beat automatically creates the remaining beats of the group.
  - Groups are tracked via `tuplet_group_id` and `tuplet_anchors`.
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
  length (up to the next onset or shorter, early stops inside a beat from
  level 4) and writes notes and rests for the whole measure: notes cross beats
  only as one value without a tie (dotted quarter or half from a beat, quarter
  or dotted quarter from the "and") inside a half bar (x/4 beats grouped like
  the eighths of n/8) or as a half centred on its boundary (`4 2 4`), never
  out of an x/8 or x/16 beat group; a note from the "and" over a beat only
  with eighths around it (no sixteenths in the beats it starts and ends in,
  the two-level rhythmic parsing rule); triplet figures
  are written as drawn. Measures are written via `set_beat`, so
  they obey the same invariants as edited ones. Seed `Rng` for reproducibility.
- Note: `Beat` `PartialEq` ignores `tuplet_group_id`. Compare the field explicitly
  if grouping matters.

## Rendering pipeline

1. `grooph_layout::pixel_layout::build_measure_layout` computes `MeasureLayout`
   (note/beam/tuplet positions) from `Measure` and `LayoutOpts`.
2. `grooph_render::measure::draw_measure` renders the result with egui.
3. The font `Bravura.otf` (SMuFL) lives in `crates/app/assets/fonts`.
4. `grooph_layout::staff_layout::build_staff_layout` lays out a score, one
   measure per row, every voice of a measure (`PlacedMeasure::voices`, one
   `MeasureLayout` each; `layout()` is the first). With several voices they
   share the line and are spaced together (`calculate_shared_x_centers`):
   beats at the same time line up and each voice keeps the room its glyphs
   need. Nothing moves a note after spacing, or the voices would part.
   `StaffOpts::voice_opts_for` gives each voice its stem direction.
   The metronome's staff is one such measure: the click bar with the counter
   rhythm as its second voice (`Grooph::metronome_staff_score`); which of
   the two the editor holds is the app's `MetronomeVoice`.
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
   (`notes::place_marks`). Every kit row starts with the percussion clef;
   the renderer draws five lines. The one line's layout is unchanged.

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
  the playback cursor, guide anchors, count labels (`Columns` in
  `grooph_render::measure`) and accuracy marks too, at the head of the beat
  standing for each onset.
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
  tabs and calibration always use the line. Not part of presets or links.
- Counting overlay comes from `grooph_measure::counting`.
- Swing (`grooph_measure::swing::Swing`, global, main menu next to BPM): 50%
  (off) to 75% (dotted), swung unit 8ths or 16ths. Notation stays straight;
  `ScoreTiming::with_swing` carries it. `performed_global_tick` moves notes and
  ghost notes in the schedule and is the reference for accuracy hits/misses;
  `written_global_tick` maps the audio position back so the cursor reaches a
  swung note when it sounds. Pairs start at the downbeat, tuplet onsets and a
  trailing incomplete pair stay straight. Not stored with library patterns.

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
  instruments `K S H HF RD CR T1 T2 FT` (`INSTRUMENT_CODES`; cymbals take two
  letters so `R` never reads as `r:`). `;` separates the voices of a
  measure. Errors name the place (`Place`: measure and voice).
- Links refuse drum kit scores (`grooph_link::one_line`: instruments or
  several voices, in `r`, `click` and `counter`) until the app shows and
  plays them (#254, #255); so does the MCP server's link check.
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
  and the MCP tool offer only `poly`: models get the notation wrong too often. A link that replaces the score
  goes through `PendingLoad::Link` and the
  unsaved-changes dialog. The share buttons (`share_link`) build a link with
  `SharedLink::to_url`.
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
  a host without it, it is the Metronome tab without the tab bar and the
  card on top: meter and tempo sit left and right of the play button, the
  options sheet has "Open in grooph" instead of share and no "Edit clicks". The theme follows
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
  rendered as a grid with counting syllables and a play link), ` ```open |
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
  changing a setting regenerates the score (undoable), 🎲 rolls a new rhythm
  with the same settings. Space and the grouping picker live in the Level
  picker, presets (load, save, update, rename, delete) in the Preset picker.
- App glue: `crates/app/src/generator.rs`. Endless mode (Bars: Endless)
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
  clicks on every beat unit of the first measure (`SoundType::CountIn` on the
  downbeat and primary beats, the quieter `CountInSoft` in between, so 7/8
  counts in all seven eighths) before the score; `Audio::playback_position` is negative meanwhile. The UI
  holds the cursor at 0 (`CountInState`) and re-anchors the accuracy session
  when the count-in ends.

- Presets (`crates/app/src/presets.rs`, UI and apply/snapshot in
  `preset_panel.rs`): `PresetSettings` = `GeneratorSettings` + BPM, swing, ghost
  notes, count-in, endless, gap click, tempo ramp and both voice sounds. Built-in
  presets are code (`builtin_presets`, addressed as `PresetId::BuiltIn(index)`),
  never stored; user presets live in `PresetLibrary` in `PersistedState`.
  `active_preset` is not persisted; at startup it is the first preset equal to
  the current settings. Applying a preset regenerates the score.

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
  (in `AudioConfig`, persisted) keeps the bars that differ from the default,
  keyed by the whole `TimeSignature` including its grouping, and applies to
  all playback in that meter. `Schedule::build` plays strong as `Downbeat`,
  medium as `PrimaryBeat`, soft as `SoftBeat` (mixer "Soft"), at the swung
  position of each note. The count-in and calibration
  ignore the click bars. Link parameter `click` is the click bar of the
  link's first meter in text notation without the time signature (e.g.
  `>q r:q. (q)`); shared links carry it when it is not the default, and
  write a picker option as `subdivision=<code>` instead (`subdivision_of`);
  a link takes one of the two.

## Metronome trainer

- `grooph_audio::{GapClick, TempoRamp}` (`crates/audio/src/trainer.rs`), carried in
  `PlaybackOptions`. Both depend only on the bar count since the score started
  (`TickSource::bars`, across loops), so nothing rebuilds the schedule.
- Gap click: the audio thread decides per bar (`GapDecider`, random mode seeded via
  `Audio::set_seed`) and filters triggers (`SoundType::muted_by`). The count-in is
  never muted. `Audio::bar_state` publishes bar + muted flag; the measure panel
  dims the muted bar.
- Tempo ramp: `TempoRamp::bpm_for_bar` from the BPM setting; the audio thread
  scales ticks per sample, the UI (`metronome_trainer.rs`) uses the same tempo in
  `score_timing()` via `PlaybackState::ramp_bpm` and re-anchors the accuracy
  session on every change.

- Metronome tab (`crates/app/src/metronome_view.rs`): its own meter
  (`MetronomeState::meter`, persisted, picked in the tab's card), separate
  from the generator's. Opening the tab puts the rhythm (score, cursor, undo,
  dirty flag) aside in `MetronomeState` and edits the meter's click bar in its
  place, so the staff, tool palette and undo work on it; `sync_click_bar`
  writes it back to `ClickBars` every frame, and closing the tab brings the
  rhythm back (persistence saves the rhythm, not the bar). The audio engine
  gets one silent bar of the meter (`metronome_score`), so only the clicks
  sound; ghost notes, endless mode and play along are off there. Below the
  staff a ring shows one segment per note or rest of the bar, as long as it,
  thickness by level; a tap cycles strong, medium, soft, off (undoable). The
  "Clicks" picker offers the ready-made bars (`click_fills`), "Edit clicks"
  shows the tool palette. A meter picked there always starts over
  (`set_metronome_meter`, undoable): its click bar is `meter_click_bar`
  (beats in x/4, every eighth or sixteenth in x/8 and x/16, stored as the
  meter's click bar) and it has no counter rhythm, whatever it had when it
  was last picked. The metronome's meter and the rhythm's never change each
  other (Practice's meter picker, Edit's time signature dialog). The staff gets a share of the free height
  (`METRONOME_STAFF_SHARE`), the ring the rest; the options sheet folds away
  like Practice's (shared `options_collapsed`). While playing, the click and beat being heard light
  up and the middle shows the bar number. Switching into or out of the tab
  stops playback.

## Play along (accuracy)

- Hit sources (`crates/app/src/play_along.rs`): MIDI NoteOns, microphone hits
  and taps (tap key, default `J`, or a press on the staff while playing). All
  hits are stamped on one clock (`crates/app/src/clock.rs`); MIDI and
  microphone timestamps are moved onto it by their age. On the web taps use the DOM event `timeStamp` (`web.rs`), natively
  the frame time. Each source has its own input offset (`PlayAlongSettings`).
- While tap capture is active, staff presses are hits, not edits.
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
  good/off windows, misses, extra hits, mean, spread) for the "Your run" window
  and the run's marks, drawn on the staff until the next run or an edit.
- Grading (`accuracy::session::HitWindows`): perfect ≤ 25 ms, good ≤ 60 ms,
  narrowed to 1/5 and 2/5 of the gap to the closest neighbouring onset (as
  heard) for fast notes. Markers are coloured by grade.
- Calibration (`crates/app/src/calibration.rs`): stashes the score and playback
  options, plays 4 quarters at 100 BPM, takes the median of the hits after a
  warm-up as the source's offset and restores the stash on stop. Persistence
  reads the stash while a calibration runs. It is started from More → Play
  along (`play_along_settings.rs`), whose dialog stays open underneath.

## Layout

- One layout for every window size, made for touch first
  (`crates/app/src/shell.rs`). On windows wider than 720 pt the controls stay
  a centered column; the staff and the tool palette use the full width.
- Tabs: Metronome (see below), Practice (generator card with
  Meter/Notes/Level/Bars/BPM, options sheet, big "new rhythm" + play
  buttons), Edit (tool palette + edit bar),
  More (`Mode::MoreMenu`, a screen of its own without the staff: a list at
  the bottom whose rows open pickers, Theme is picked in its row). The tab is
  derived from `Mode` (`tab`), so modes set elsewhere land on the right tab;
  the Metronome tab is open while `metronome_mode()`, in `Mode::Playback`
  (ring) or `Mode::Edit` (tool palette). More stays in front of an open
  metronome and leaves it playing (`advance_unseen_playback` moves the
  playback on without a staff); saving or loading in the library puts it
  away first (`leave_metronome`), and a loaded pattern opens on the rhythm
  page More was opened from (`ShellState::rhythm_tab`).
- The library (`library_panel.rs`) has two halves, Rhythms and Presets, of
  the same cards (`library_card`): a tap loads a rhythm or applies a preset
  and opens the practice page (`open_preset`), ✏ renames or deletes, built-in
  presets have no ✏. The Preset picker on the practice page shows the same
  preset cards (`preset_cards`); a tap there applies the preset and stays.
- Settings open in a `Modal` picker (`settings_picker`); generator changes go
  through `finish_generator_change`.
- The staff's em is capped so one row fits the panel's height. The row's
  height is measured from its laid-out notes (`StaffLayout::room`,
  `RowRoom::height`).
- Windows narrower than `NARROW_MAX_WIDTH` (600 pt, `UiShell::narrow_screen`)
  get tighter staff spacing, so glyphs are bigger on phones.
- While a tempo ramp runs, the card shows the tempo being played ("BPM now").
- Share (right of play, and in the edit bar) builds a score link with tempo
  and swing; on the web it opens the system share sheet
  (`platform::open_share_sheet`, Web Share API), else copies the link. In the
  Metronome tab, share (right of play) builds a `ts` link with tempo, swing
  and the click bar instead.
- Keyboard shortcuts are ignored while a text field has the focus.


- Tool registry: `crates/app/src/tools.rs` (ToolKind, Modifier, shortcuts).
- Keyboard handling: `crates/app/src/keyboard_input.rs`.
- Measure operations: `grooph_measure::editing::{Modification, set_tuplet, ...}`.

If you add new tools/shortcuts, remember:
- Tool registry + palette
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
- Tuplet tests: note auto-fill and `tuplet_group_id` comparisons.

## Change navigation (quick)

- UI/State/UX: `crates/app/src/lib.rs` and panels (`*_panel.rs`).
- Model/rules: `crates/measure/src/lib.rs`, `editing.rs`, `fill.rs`.
- Layout/geometry: `crates/layout/src/pixel_layout.rs`, `render_plan.rs`.
- Drawing: `crates/render/src/measure.rs`, `beat.rs`.
- Audio: `crates/audio/src/lib.rs`.
- MIDI: `crates/midi/src/input.rs`.
- Microphone: `crates/mic/src/` (detector, `native.rs`, `web.rs`).

## Notes for LLM agents

- Model changes can affect layout, audio, and rendering.
  Search for `DEFAULT_GRID` usage and tick logic.
- Web-specific logic is gated by `cfg(target_arch = "wasm32")`.
- Keep measure validation in mind: `set_beat`/`set_tuplet` must not leave
  unfillable measure lengths.
