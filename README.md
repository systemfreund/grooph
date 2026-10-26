# grooph

**A free rhythm trainer: read rhythms, keep time, and see how well you play
them.** grooph runs in your browser at **[grooph.app](https://grooph.app)**,
with no account and no installation, and as an Android app.

<img width="945" height="2048" alt="grooph on a phone" src="https://github.com/user-attachments/assets/208b524b-b48e-4410-8b41-dfb3f060c8e7" />

## What you can do with it

### Practice sight-reading

- Generate random rhythms in standard notation: eighths, sixteenths,
  triplets or a mix, in five difficulty levels.
- Pick 1 to 8 bars or play in endless mode, where new bars keep coming.
- Choose the time signature, including odd meters like 5/4, 7/8 or 9/8.
- Hand-pick the rhythm figures you want to work on, add rests, and save your
  settings as presets.
- Show counting syllables under the notes (numbers, Takadimi or Gordon) and
  let the staff flash on the beats instead of following a cursor.

### Use the metronome

- Any tempo from 20 to 300 BPM and any time signature with its own beat
  groups and accents.
- Decide what clicks on every beat and subdivision: strong, medium, soft or
  silent.
- Play a polyrhythm against the click, such as 3:2, 5:4 or a ratio of your
  own.
- Swing from straight to dotted feel, a count-in, and a choice of sounds
  (claves, snare, xylophone, click).
- Trainer modes: bars where the metronome goes silent to test your inner
  time, and a tempo that rises step by step.

### Play along and check your timing

- Tap on the keyboard or touchscreen, play sticks or a practice pad into the
  microphone, or connect a MIDI drum kit or keyboard.
- Every hit is marked as early, late or on time, and missed notes are shown.
  After a run you get a summary of how you did.
- A short calibration compensates for the delay of your device.

### Write and share rhythms

- Write your own rhythms with notes, rests, dots, accents, ghost notes
  and tuplets, and keep them in your library.
- Share a rhythm, an exercise or a metronome setting as a link.

## Learn more

The guides on grooph.app explain the features and the basics of reading
rhythms:

- [Rhythm generator](https://grooph.app/rhythm-generator/) and
  [sight-reading exercises](https://grooph.app/sight-reading/)
- [Metronome](https://grooph.app/metronome/),
  [polyrhythms](https://grooph.app/metronome/polyrhythms/),
  [odd time signatures](https://grooph.app/metronome/odd-time-signatures/),
  [swing](https://grooph.app/metronome/swing/) and the
  [trainer modes](https://grooph.app/metronome/trainer/)
- [Timing practice](https://grooph.app/timing-practice/) with tapping,
  microphone or MIDI
- [Learn to count](https://grooph.app/learn/) sixteenths, triplets,
  syncopation and dotted rhythms

## Privacy

grooph has no accounts and no tracking. Your settings and rhythms stay on
your device, and microphone audio is analysed locally and never sent
anywhere. Details are in the [privacy notice](https://grooph.app/privacy/).

## Feedback

Found a bug or missing a feature? Please
[open an issue](https://github.com/systemfreund/grooph/issues).

## Building it yourself

grooph is written in Rust with [egui](https://github.com/emilk/egui):

- Run the desktop app: `cargo run` (on Linux, install the ALSA headers
  first, e.g. `libasound2-dev`)
- Run the web app: `trunk serve`
- Run the tests: `cargo test --workspace`

[ARCHITECTURE.md](ARCHITECTURE.md) describes how the code is organised,
including the Android build, and [AGENTS.md](AGENTS.md) the development
and review practices.

## License

grooph is licensed under the [GNU General Public License v3.0](LICENSE).
Some sound samples have licenses of their own, listed in
[crates/audio/samples/CREDITS.md](crates/audio/samples/CREDITS.md).
