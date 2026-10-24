---
title: Polyrhythm Metronome
heading: Polyrhythm metronome – practice 3 against 2, 3 against 4 and more
description: A free online metronome that plays a polyrhythm against its click: 3:2, 3:4, 5:4, 4:3 or any ratio of your own. Open it with a link.
section: Metronome
order: 5
---
A polyrhythm is two even pulses at once that do not divide into each other: three notes in
the time of two, four in the time of three. grooph's metronome can play such a **counter
rhythm** against its normal click, with a sound of its own, so you hear exactly how the two
pulses line up.

```open | Open the metronome with 3 against 2 in 4/4
bpm=90&ts=4/4&counter=t4+t4+t4+t4+t4+t4
```

## How to use it

1. Open the **Metronome** tab and pick a time signature.
2. Tap **Polyrhythm** and choose a ratio. **3:2** means three counter clicks in the time of
   two beats; the pattern repeats until the bar is full.
3. Under **Your own (clicks : beats)** you can set any other ratio that fits the meter,
   for example 7:4 or 9:4.
4. Press play. The counter rhythm has its own volume in the mixer (speaker icon), and you
   can tap single counter clicks on the circle to make them louder, softer or silent.

The beats count in the meter's beat unit: quarters in 4/4 and 3/4, eighths in 6/8 or 7/8.

## Ready-made polyrhythms

These links open the metronome with the polyrhythm already switched on:

```open | 3:4 in 4/4 – three against four
bpm=80&ts=4/4&counter=t2+t2+t2
```

```open | 5:4 in 4/4 – five against four
bpm=70&ts=4/4&counter=qt4+qt4+qt4+qt4+qt4
```

```open | 2:3 in 3/4 – two against three
bpm=90&ts=3/4&counter=q.+q.
```

```open | 4:3 in 3/4 – four against three
bpm=80&ts=3/4&counter=e.+e.+e.+e.
```

```open | 2:3 in 6/8 – duplets against the dotted quarters
bpm=60&ts=6/8&counter=e.+e.+e.+e.
```

```open | 7:5 in 5/8 – seven eighths in the time of five
bpm=80&ts=5/8&counter=7:5e+7:5e+7:5e+7:5e+7:5e+7:5e+7:5e
```

## Building your own link

The `counter` parameter is one bar of the meter in grooph's
[text notation](https://grooph.app/llms.txt): the counter clicks as notes, rests where it is
silent. A note with `>` clicks strong, a note in parentheses such as `(e)` soft. It needs
`ts` and works only for the metronome, not together with a written rhythm. The easiest way
to get a link is to set the polyrhythm in the app and use **Copy link** in the library panel.

A ratio with a common factor sounds like the reduced one: 4:2 in 4/4 is four eighths in the
time of two quarters, the same as 2:1, so a 4:2 link opens as 2:1:

```open | 2:1 (or 4:2) in 4/4 – eighths against quarters
bpm=90&ts=4/4&counter=e+e+e+e+e+e+e+e
```

## FAQ

### How do I practice three against two?

Start slowly and count the six eighth-note triplets of two beats: "1 2 3 4 5 6". The click
falls on 1 and 4, the counter rhythm on 1, 3 and 5. Clap the counter rhythm while the click
plays, then swap hands.

### Which ratios does grooph offer?

Every ratio of 2 to 16 clicks over a number of beats that divides the bar, as long as the
clicks land on note values grooph can write: 3:2, 3:4, 5:4 and 7:4 in 4/4, 2:3 and 4:3 in
3/4, 3:5, 4:5 and 7:5 in 5/4, and many more.

### Can I send someone a polyrhythm?

Yes. **Copy link** in the library panel copies a link with the tempo, the meter, the click
and the polyrhythm.
