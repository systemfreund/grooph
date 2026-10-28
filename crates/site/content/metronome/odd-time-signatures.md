---
title: Metronome for Odd Time Signatures
heading: A metronome for odd time signatures like 5/4, 7/8 and 11/8
description: Practice 5/4, 5/8, 7/8, 9/8 and other odd meters with a free online metronome. Learn the common groupings and hear them clicked.
section: Metronome
order: 2
---
Odd time signatures – meters such as 5/4, 7/8 or 11/8 – feel unusual because their beats do
not split into equal halves. The trick is to think of them as short groups of **twos and
threes**. grooph lets you set meters from 1/4 to 7/4, 3/8 to 13/8 and 5/16 to 19/16 and
hear exactly the grouping you want.

## Quick links

```open | 5/4 at 120 BPM
bpm=120&ts=5/4
```

```open | 5/8 at 80 BPM
bpm=80&ts=5/8
```

```open | 7/8 at 90 BPM
bpm=90&ts=7/8
```

```open | 11/8 at 100 BPM
bpm=100&ts=11/8
```

## Counting in twos and threes

In eighth-note meters (5/8, 7/8, 9/8, …) the beats are usually grouped. A group of two is
counted "1 2", a group of three "1 2 3". The first note of each group gets a little more
weight. The same meter can be grouped in different ways, and the grouping changes the feel
completely.

| Meter | Common groupings |
|---|---|
| 5/8 | 3+2, 2+3 |
| 7/8 | 2+2+3, 3+2+2, 2+3+2 |
| 9/8 | 3+3+3 (compound), 2+2+2+3 |
| 11/8 | 3+3+3+2, 2+2+3+2+2 |

grooph draws 5/8 as 3+2 and 7/8 as 2+3+2 by default. To practice another grouping, pick it
next to the meter in the time signature dialog: the beams, the beat ring, the click and the
counting then follow the groups, and the first note of each group clicks a little louder.
In a link the grouping is its own parameter, `ts=7/8&groups=2+2+3`:

```open | 7/8 grouped 2+2+3 at 90 BPM
bpm=90&ts=7/8&groups=2%2B2%2B3
```

```open | 7/8 grouped 3+2+2 at 90 BPM
bpm=90&ts=7/8&groups=3%2B2%2B2
```

```open | 5/8 grouped 2+3 at 80 BPM
bpm=80&ts=5/8&groups=2%2B3
```

In a written rhythm the grouping comes in front of the meter, and accents make the groups
stand out even more:

```rhythm bpm=90 | 7/8 grouped 2+2+3
2+2+3/8 >e e >e e >e e e
```

```rhythm bpm=90 | 7/8 grouped 3+2+2
3+2+2/8 >e e e >e e >e e
```

## 5/4 in quarter notes

5/4 is counted in quarter notes, "1 2 3 4 5". Many players feel it as 3+2: a waltz bar
followed by a short bar of two.

```rhythm bpm=120 | 5/4 felt as 3+2
5/4 >q q q >q q
```

## How to practice odd meters

1. **Clap the groups first.** Say "1 2, 1 2, 1 2 3" while clapping only on every "1".
2. **Use a slow tempo.** The BPM always counts quarter notes, so 90 BPM in 7/8 plays the
   eighths at 180 per minute. Start where the eighths feel easy, then speed up.
3. **Write your own figures.** Open the editor in grooph, change the time signature and
   fill the measure with the rhythm of your piece.

## FAQ

### What is the difference between 6/8 and 3/4?

Both have six eighth notes per measure. 3/4 is felt as three quarter notes (2+2+2), 6/8
as two dotted quarters (3+3).

### Which meters does grooph support?

1 to 7 beats over a quarter note, 3 to 13 over an eighth and 5 to 19 over a sixteenth. The
random rhythm generator writes exercises in all of them, grouped 3+2, 3+3, 2+3+2 and so
on.
