// The metronome link of the `show_metronome` tool: the tool's arguments are
// the app's link parameters (see crates/link and llms.txt), so the server and
// the view build the same `?v=2&ts=…` query from them. `metronomeQuery` is
// also inlined into the view (view.js), so it must stay self-contained.

/** Query (without `?`) that opens the metronome as `args` describe, written
 * as the app's share links are (crates/link `SharedLink::to_url`). */
export function metronomeQuery(args) {
  const encode = (value) =>
    Array.from(new TextEncoder().encode(value), (b) => {
      const c = String.fromCharCode(b);
      if (/[A-Za-z0-9\-_.~/:]/.test(c)) return c;
      return b === 32 ? "+" : "%" + b.toString(16).toUpperCase().padStart(2, "0");
    }).join("");
  const text = (value) => (value === undefined || value === null ? "" : String(value).trim());
  const params = [["v", "2"]];
  if (text(args?.bpm)) params.push(["bpm", text(args.bpm)]);
  params.push(["ts", text(args?.ts) || "4/4"]);
  if (text(args?.swing)) params.push(["swing", text(args.swing)]);
  if (text(args?.su)) params.push(["su", text(args.su)]);
  if (text(args?.click)) params.push(["click", text(args.click)]);
  if (text(args?.counter)) params.push(["counter", text(args.counter)]);
  return params.map(([key, value]) => `${key}=${encode(value)}`).join("&");
}

/** Why `args` cannot be a metronome link, or `null`. Checks the numbers;
 * the notation of `ts`, `click` and `counter` is checked by the app, which
 * names the problem in the view. */
export function argumentError(args) {
  const whole = (value, min, max) => Number.isInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
  if (args?.bpm != null && !whole(args.bpm, 20, 300)) return `bpm must be a whole number from 20 to 300, got ${args.bpm}`;
  if (args?.swing != null && !whole(args.swing, 50, 75)) return `swing must be a whole number from 50 to 75, got ${args.swing}`;
  if (args?.su != null && !["8", "16"].includes(String(args.su))) return `su must be 8 or 16, got ${args.su}`;
  if (args?.su != null && args?.swing == null) return "su needs a swing amount (swing 50-75)";
  if (args?.ts != null && !/^\s*\d+(\+\d+)*\/\d+\s*$/.test(String(args.ts))) return `ts must be a time signature such as 7/8, got ${args.ts}`;
  return null;
}

/** Counter rhythms of common polyrhythms, as in llms.txt ("Polyrhythms"),
 * for the tool's description. */
export const POLYRHYTHMS = [
  { ts: "4/4", ratio: "3:2", counter: "t4 t4 t4 t4 t4 t4" },
  { ts: "4/4", ratio: "3:4", counter: "t2 t2 t2" },
  { ts: "4/4", ratio: "5:4", counter: "qt4 qt4 qt4 qt4 qt4" },
  { ts: "4/4", ratio: "7:4", counter: "spt4 spt4 spt4 spt4 spt4 spt4 spt4" },
  { ts: "4/4", ratio: "2:1", counter: "e e e e e e e e" },
  { ts: "2/4", ratio: "3:2", counter: "t4 t4 t4" },
  { ts: "3/4", ratio: "2:3", counter: "q. q." },
  { ts: "3/4", ratio: "4:3", counter: "e. e. e. e." },
  { ts: "6/8", ratio: "2:3", counter: "e. e. e. e." },
  { ts: "6/8", ratio: "4:3", counter: "s. s. s. s. s. s. s. s." },
];
