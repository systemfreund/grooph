# grooph in Claude's Connectors Directory

Everything to paste into the developer portal ([claude.ai/directory/manage](https://claude.ai/directory/manage) →
**Submit new** → **MCP connector**) for issue #230. The portal's limits are noted with each field.

## Connection

- **Server URL:** `https://grooph.app/mcp` (one URL for everyone; keep it for good)
- **Authentication:** none. Everything the tool does is public.

## Listing

**Name** (≤ 100): `grooph`

**Slug** (permanent once published): `grooph` → `https://claude.ai/directory/connectors/grooph`

**One-liner** (≤ 200):

> A metronome in the chat: any tempo and time signature, accents, swing and polyrhythms, ready to play.

**Description** (≤ 2,000):

> grooph shows a working metronome right inside the conversation. Ask for the click you need and Claude sets it up: any tempo from 20 to 300 bpm, time signatures from 1/4 to 7/4, 3/8 to 13/8 and 5/16 to 19/16, beat groupings such as 2+2+3 in 7/8, subdivisions from eighths to triplets, quintuplets and thirty-seconds, a click bar of your own with accents, ghost notes and rests, swing from straight to dotted, and polyrhythms such as 3:2, 5:4 or 7:5 against the beat.
>
> The metronome appears as a compact card with the tempo, the click on a staff and the beats of the bar on a ring. Tap play to start it. Fullscreen shows all of grooph's metronome controls, and the changes you make there come back to Claude, so you can keep refining the setting in the conversation ("a bit faster", "now with swing"). "Open in grooph" opens the same metronome in the full app at grooph.app, which adds a rhythm generator for sight-reading, timing practice and a library of your own rhythms.
>
> No account is needed, neither for grooph nor for the connector. The connector stores nothing and only receives the metronome settings Claude chooses.

**Categories** (1 to 5, pick from the portal's list): music, education, productivity

**Documentation URL:** `https://grooph.app/claude/`

**Privacy policy URL:** `https://grooph.app/privacy/`

**Support contact:** `hello@grooph.app`

**Icon:** `crates/android/store/icon-512.png` (512 × 512 PNG, the app icon)

**Allowed link URIs:** `https://grooph.app` (so "Open in grooph" opens without a confirmation prompt)

## Use cases

1. **Practise with a click** at a given tempo and meter: "Give me a metronome at 90 bpm in 4/4."
2. **Odd meters with their grouping:** "I need a 7/8 click at 140 eighths per minute, grouped 2+2+3."
3. **Polyrhythm practice:** "Play a 3:2 polyrhythm against 4/4 at 80."
4. **Swing feel:** "A swing click with triplet feel at 120."

- **Prerequisites:** none (no account, no setup beyond adding the connector).
- **Data:** reads nothing from the user's accounts and writes nothing; the tool is read-only.

## Data handling

- API: first-party (grooph's own server on grooph.app serves grooph's own app).
- No personal health data, no sponsored content, no financial transactions, no AI media generation.

## Screenshots

PNG files in [`screenshots/`](screenshots/), 1000 px wide or more, cropped to the app's response. Each one with its
exact prompt:

| File | Prompt |
| --- | --- |
| `1-four-four.png` | Give me a metronome at 90 bpm in 4/4. |
| `2-seven-eight.png` | I need a 7/8 click at 140 eighths per minute, grouped 2+2+3. |
| `3-polyrhythm.png` | Play a 3:2 polyrhythm against 4/4 at 80. |
| `4-swing.png` | A swing click with triplet feel at 120. |
| `5-fullscreen-dark.png` | I need a 7/8 click at 140 eighths per minute, grouped 2+2+3. (fullscreen, dark mode) |

They are taken with `screenshots.mjs` against the local MCP Apps host (`mcp/local-host.mjs`), which runs the same
server and view as grooph.app:

```sh
trunk build --release
node mcp/local-host.mjs &
node docs/claude-directory/screenshots.mjs
```

## Reviewer instructions (test & launch)

> No account or credentials are needed. Add `https://grooph.app/mcp` as a connector and try these prompts:
>
> 1. Give me a metronome at 90 bpm in 4/4.
> 2. I need a 7/8 click at 140 eighths per minute, grouped 2+2+3.
> 3. Play a 3:2 polyrhythm against 4/4 at 80.
> 4. A swing click with triplet feel at 120.
>
> Each prompt shows an interactive metronome in the chat. The click starts only when you tap play (browsers do not allow sound without a tap). On an iPhone in the Claude app, the click is silent while the ring/silent switch is on, because the app's web view follows that switch; please switch silent mode off before testing sound there. Invalid settings (for example "a 7:5 polyrhythm in 4/4") come back as an error that names the problem, and Claude corrects them. The full app is at https://grooph.app.

## Note for Anthropic (mcp-review@anthropic.com)

> In the Claude iOS app, an MCP App cannot play sound while the ring/silent switch is on: the native WKWebView sets the audio session category, so `navigator.audioSession.type = "playback"` in the view has no effect. A metronome needs the "playback" category; it would help if the iOS app's MCP Apps web view used it (or let a view ask for it).

## Before submitting

- [ ] The privacy page and `https://grooph.app/claude/` are live.
- [ ] Every tool was run in the MCP Inspector (CI does this after every deploy, `mcp/smoke.mjs`) and as a custom connector in Claude (Settings → Connectors → Add custom connector).
- [ ] After listing: check the Community label, install it from the directory on web and on a phone, and add the directory link to `crates/site/content/claude.md`.
