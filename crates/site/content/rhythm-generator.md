---
title: Random Rhythm Generator
heading: Random rhythm generator for sight-reading practice
description: Generate random rhythms in standard notation for sight-reading – eighths, sixteenths, triplets or mixed, five difficulty levels, played back with a metronome.
section: Features
order: 1
---
Reading the same exercise twice is memorizing, not reading. grooph's rhythm generator
writes a **new rhythm in standard notation** every time you ask, plays it back with a
metronome and a moving cursor, and lets you choose exactly how hard it should be.

```open | Generate a sixteenth-note rhythm (level 3, 4 bars)
bpm=80&sub=16&lvl=3&bars=4
```

## Settings

The generator is the **Practice** tab. The card at the top shows meter, notes, level, bars
and tempo; tap a value to change it. Every change creates a new rhythm, and the dice button
rolls a new one with the same settings.

| Setting | What it does |
|---|---|
| **Notes** | The finest note value: 8th notes, 16th notes, triplets, or mixed (16ths and triplets together). Eighth-note meters offer 8ths and 16ths, sixteenth-note meters 16ths. |
| **Level** | Level 1 to 5. Higher levels add syncopations, rests inside the beat and offbeat figures. |
| **Bars** | 1 to 8 measures per exercise. |
| **Meter** | Quarter-note meters from 1/4 to 7/4, eighth-note meters from 3/8 to 13/8 and sixteenth-note meters from 5/16 to 19/16. |
| **Space** | Adds whole-beat rests, from none to a lot. Good for practicing silence. In the **Level** dialog. |
| **Groupings** | Instead of a level, pick exactly which one-beat figures may appear. In the **Level** dialog. |
| **Endless** | Keeps replacing bars you have already played, so the exercise never repeats. |
| **Ghost notes** | Quiet clicks on every free slot of the subdivision, to hear the grid. |
| **Preset** | Recall a saved exercise. |

## Presets

A preset stores the generator settings together with everything you practise with them:
tempo, swing, ghost notes, count-in, endless mode, gap click, tempo ramp and the sounds.
Tap **Preset** and pick one to restore all of it at once. In the same dialog you save the
current settings as a new preset, update the active one, and rename or delete your own with
the pencil button.
A few built-in presets get you started, for example "16th funk" or "Speed builder"; they
cannot be changed, but you can save a copy under a new name.

## How the levels work

Each beat of the measure is one short figure, called a *grouping*. A level unlocks a set
of groupings; the generator favors the newest ones, but easier figures stay in the mix.

```rhythm | Level 1: quarters, quarter rests and pairs of eighths
4/4 q e e r:q e e
```

```rhythm | Level 2: the first offbeat eighths
4/4 r:e e q e e r:e e
```

```rhythm | Level 3 with sixteenths: figures that start on the beat
4/4 s s s s e s s s s e q
```

```rhythm | Level 5 with sixteenths: notes on "e" and "a"
4/4 r:s s e r:s s s s r:s e. q
```

## Eighth-note meters

In 6/8, 9/8 and 12/8 a beat is a dotted quarter: three eighths. The generator writes one
figure per group of three, from the plain dotted quarter and three eighths up to
sixteenths and rests inside the group. With 8th notes the levels follow the triplet
ladder: three eighths first, then quarter + eighth, then figures that start later.

```rhythm | 6/8, level 2 with 8th notes
6/8 e e e q e
```

Odd meters mix groups of two and three eighths: 5/8 is counted 3+2, 7/8 2+3+2, 8/8
3+3+2, 10/8 3+3+2+2, 11/8 3+3+3+2 and 13/8 3+3+3+2+2. Each
group of two gets the same figures as a quarter-note beat, each group of three the
dotted-quarter figures.

```open | Generate a 7/8 rhythm with sixteenths (level 3)
bpm=45&ts=7/8&sub=16&lvl=3&bars=2
```

Sixteenth-note meters such as 7/16 or 11/16 work the same way one note value down: groups
of two sixteenths (an eighth) and three sixteenths (a dotted eighth), always written in
sixteenths.

The guides for [eighth notes](/sight-reading/eighth-notes/),
[sixteenth notes](/sight-reading/sixteenth-notes/) and [triplets](/sight-reading/triplets/)
list every level in detail.

## Share an exercise

Every exercise can be shared as a link. A link with a **seed** always produces the same
rhythm, so a teacher can send the same exercise to a whole class:

```open | The same 2-bar triplet exercise for everyone
bpm=72&sub=3&lvl=2&bars=2&seed=2024
```

In the library (**More → Library**), **Copy link** copies a link to whatever is on the screen, including
rhythms you wrote yourself.

## FAQ

### Is the rhythm generator free?

Yes. It runs in your browser without an account.

### Can I print the exercises?

Not yet. You can share them as links or save them in the library.

### Which time signatures does the generator use?

Every meter the editor offers: 1/4 to 7/4, 3/8 to 13/8 and 5/16 to 19/16. Eighth- and
sixteenth-note meters are written in their usual groups of two and three.
