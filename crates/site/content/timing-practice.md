---
title: Timing Practice with MIDI
heading: Check your timing with a MIDI drum pad or keyboard
description: Play along with a rhythm on a MIDI drum pad or keyboard and see how early or late every hit is. Free timing and accuracy practice in your browser.
section: Features
order: 2
---
A metronome tells you where the beat is, but not whether you hit it. Connect a MIDI
device – an electronic drum kit, a practice pad with MIDI, a keyboard or a pad controller –
and grooph shows for every note how **early or late** you played it and which notes you
missed.

```open | Practice timing with a sixteenth-note exercise
bpm=70&sub=16&lvl=2&bars=2
```

## Setup

1. Connect the MIDI device to your computer (USB or Bluetooth MIDI).
2. Open grooph in a browser with Web MIDI support, such as Chrome or Edge.
3. Open the settings (gear icon) and choose the device under **MIDI Input**.
4. Make sure accuracy tracking (target icon) is switched on, then press play.

Any key or pad counts as a hit, so you can use whichever sound you like.

## Reading the result

Each written note gets a marker. A marker left of the note means you played early, right
of it late. Notes you did not play are shown as misses. Try to keep the markers close to
the notes for a whole exercise before you raise the tempo.

## Exercises

Start with a plain pulse, then make it harder:

```rhythm bpm=80 | Eighth notes – keep them even
4/4 e e e e e e e e
```

```rhythm bpm=80 | Offbeats only
4/4 r:e e r:e e r:e e r:e e
```

```rhythm bpm=70 | Sixteenths with an accent on every beat
4/4 >s s s s >s s s s >s s s s >s s s s
```

The [rhythm generator](/rhythm-generator/) writes endless new material at any level.

## Tips

- **Turn on the count-in** so that the first note is not a guess.
- **Practice slowly.** Accuracy at 60 BPM is worth more than a sloppy 120.
- **Watch the trend, not single hits.** If most markers are early, you are rushing.

## FAQ

### Do I need a MIDI device?

For the timing feedback, yes. Everything else in grooph – metronome, editor and generator –
works without one.

### Does it work in Safari or Firefox?

Timing feedback needs Web MIDI, which Chrome and Edge support. Other browsers may not
offer it yet.
