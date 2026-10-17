---
title: Timing Practice
heading: Check your timing by tapping, with sticks or a MIDI drum pad
description: Tap along, play sticks into the microphone or use a MIDI drum pad and see how early or late every hit is. Free timing practice in your browser.
section: Features
order: 2
---
A metronome tells you where the beat is, but not whether you hit it. Tap along on your
keyboard or touchscreen, play sticks or a practice pad into the microphone, or connect a
MIDI device – an electronic drum kit, a practice pad with MIDI, a keyboard or a pad
controller – and grooph shows for every note how **early or late** you played it and
which notes you missed.

```open | Practice timing with a sixteenth-note exercise
bpm=70&sub=16&lvl=2&bars=2
```

## Setup

### Tapping

1. Make sure **Timing feedback** on the **Practice** tab is switched on, then press play.
2. Tap every note: press **J** on the keyboard, or touch or click the staff.

While you play along, touching the staff counts as a hit and does not move the cursor.
You can pick another tap key or switch tapping off under **Play Along** in the settings.

### Microphone

1. Open **More → Settings**, go to **Play Along** and switch on **Microphone input**. Allow
   grooph to use the microphone when the browser asks.
2. Hit your sticks, pad or hands and watch the level meter: hits should cross the red
   threshold mark, the click on its own should stay below it. Move the threshold if not.
3. Make sure **Timing feedback** on the **Practice** tab is switched on, then press play.

Sharp sounds work best: sticks on sticks, a practice pad, a rim or claps. The microphone
only listens while you play along. On an iPhone or iPad with a headset, grooph listens
through the device's own microphone, because headset microphones filter out hits.

### MIDI

1. Connect the MIDI device to your computer (USB or Bluetooth MIDI).
2. Open grooph in a browser with Web MIDI support, such as Chrome or Edge.
3. Open **More → Settings** and choose the device under **MIDI Input**.
4. Make sure **Timing feedback** on the **Practice** tab is switched on, then press play.

Any key or pad counts as a hit, so you can use whichever sound you like.

## Calibrate the latency

Every device adds a little delay between your hit and the moment grooph sees it, so your
hits can look late although they are not. Open **More → Settings**, go to
**Play Along** and press **Calibrate…** next to *Tap*, *MIDI* or *Mic*. grooph plays a
click and you play along on every beat; after 16 hits the median deviation becomes the
offset that is subtracted from your hits. Every input has its own offset. Bluetooth
headphones add a lot of delay, so calibrate again when you switch to or from them.

## Reading the result

Each written note gets a marker. A marker left of the note means you played early, right
of it late. Blue markers are perfect and orange ones good; hits that are off are purple and
get an arrow pointing the way you were off. Notes you did not play get a purple cross. The
colours are chosen to stay apart with colour vision deficiencies. The markers stay on the staff after you stop, so you can see
where the run went wrong, until you play again or edit the rhythm. Try to keep the markers
close to the notes for a whole exercise before you raise the tempo.

When you stop, grooph sums up the run: a score in percent, how many hits were perfect
(within 25 ms), good (within 60 ms) or off, missed notes, extra hits, your average deviation
and the spread. Fast notes get tighter windows, at most a fifth (perfect) and two fifths
(good) of the gap to the neighbouring note, so a hit can't be "good" when it is nearly on
the next note. While you play, the score and the average deviation are shown above the
play button, which is handy in endless mode.

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

No. You can tap along on the keyboard or the touchscreen, or play sticks or a practice
pad into the microphone. A MIDI pad gives the cleanest signal.

### Does it work in Safari or Firefox?

Tapping and the microphone work in every current browser. MIDI needs Web MIDI, which
Chrome and Edge support; other browsers may not offer it yet.
