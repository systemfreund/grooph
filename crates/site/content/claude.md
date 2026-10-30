---
title: grooph in Claude
heading: A metronome in Claude – grooph's connector for AI chats
description: Ask Claude for a click in any tempo and meter and get a working metronome in the chat, with groupings, subdivisions, swing and polyrhythms. No account needed.
section: Features
order: 3
---
With the grooph connector, Claude can show grooph's metronome **right inside the chat**.
Ask for "a 7/8 click at 140 eighths per minute, grouped 2+2+3" or "3:2 against 4/4 at
80", and a working metronome appears in the conversation: tap play and practise. Ask for
a change ("a bit faster", "now with swing") and Claude updates it. You need no account,
neither for grooph nor for the connector.

```open | The same metronome in grooph: 3:2 against 4/4 at 90
bpm=90&ts=4/4&poly=3:2
```

## Add it to Claude

The connector works on claude.ai, in the Claude desktop and mobile apps, in Claude Code
and in Cowork.

1. In Claude, open **Settings → Connectors**.
2. Choose **Add custom connector**.
3. Enter the URL `https://grooph.app/mcp` and add it. No sign-in is needed.

Then ask Claude for a metronome in any chat. Other chat apps that support
[MCP Apps](https://modelcontextprotocol.io/extensions/apps/overview) can use the same URL.

## Example prompts

- "Give me a metronome at 90 bpm in 4/4."
- "I need a 7/8 click at 140 eighths per minute, grouped 2+2+3."
- "Play a 3:2 polyrhythm against 4/4 at 80."
- "A swing click with triplet feel at 120."
- "Now group it 3+2+2 and make it a bit faster."

## What you see

The metronome appears as a compact card in the chat: the tempo, the click on a staff and
the beats of the bar on a ring with the play button in the middle. The click starts only
when you tap play. Fullscreen shows everything the metronome can do; changes you make
there (tempo, meter, clicks) are passed back to Claude, so it can refer to them. **Open in grooph** opens the
same setting in the full app.

On an **iPhone** in the Claude app, the click is silent while the ring/silent switch is
on, because the Claude app decides how its web views play sound. Switch silent mode off
to hear it. In Safari and on other devices this does not apply.

## The tool

The connector has one tool, `show_metronome`. It is read-only: it shows a metronome and
changes nothing. Its arguments are the metronome's own settings:

| Argument | What it sets |
| --- | --- |
| `bpm` | Tempo, 20 to 300 quarter notes per minute (in x/8 the eighths count double). Default 120. |
| `ts` | Time signature: 1–7 beats over 4, 3–13 over 8 or 5–19 over 16. Default 4/4. |
| `groups` | How the beats group, e.g. `3+2+2` in 7/8. |
| `subdivision` | Clicks between the beats, e.g. eighths, triplets or sixteenths. |
| `click` | A click bar of its own in grooph's rhythm notation: rests, accents, ghost notes. |
| `poly` | A polyrhythm against the click, e.g. `3:2` or `5:4`. |
| `swing`, `su` | Swing from 50 (straight) to 75 (dotted), on eighths or sixteenths. |
| `link` | A metronome already shown, to change only some of its settings. |

Settings that do not fit together, such as a click bar that is too long for the meter,
come back to Claude as an error that names the problem, so it can correct them.

## Privacy

The connector receives only the metronome settings Claude chooses and stores nothing.
The metronome in the chat keeps no data and does not use the microphone; it tells Claude
its current settings so the conversation can refer to them. Details are in the
[privacy policy](/privacy/).

## Support

Questions, problems or ideas: [hello@grooph.app](mailto:hello@grooph.app).
