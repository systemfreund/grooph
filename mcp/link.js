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
  // Eighths swing by default: the app writes su only for sixteenths.
  if (text(args?.su) && text(args.su) !== "8") params.push(["su", text(args.su)]);
  if (text(args?.click)) params.push(["click", text(args.click)]);
  if (text(args?.poly)) params.push(["poly", text(args.poly).replace(/\s+/g, "")]);
  return params.map(([key, value]) => `${key}=${encode(value)}`).join("&");
}

/** Why `args` cannot be a metronome link, or `null`. Checks that only the
 * `known` arguments are given and the numbers; the notation of `ts` and
 * `click`, and whether `poly` fits the meter, are checked by the app, which
 * names the problem in the view. */
export function argumentError(args, known) {
  const unknown = Object.keys(args || {}).filter((key) => !known.includes(key));
  if (unknown.includes("counter")) return "counter is not an argument: pass the polyrhythm as poly, e.g. \"3:2\"";
  if (unknown.length) return `unknown argument ${unknown.join(", ")}; the arguments are ${known.join(", ")}`;
  const whole = (value, min, max) => Number.isInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
  if (args?.bpm != null && !whole(args.bpm, 20, 300)) return `bpm must be a whole number from 20 to 300, got ${args.bpm}`;
  if (args?.swing != null && !whole(args.swing, 50, 75)) return `swing must be a whole number from 50 to 75, got ${args.swing}`;
  if (args?.su != null && !["8", "16"].includes(String(args.su))) return `su must be 8 or 16, got ${args.su}`;
  if (args?.su != null && args?.swing == null) return "su needs a swing amount (swing 50-75)";
  if (args?.ts != null && !/^\s*\d+(\+\d+)*\/\d+\s*$/.test(String(args.ts))) return `ts must be a time signature such as 7/8, got ${args.ts}`;
  if (args?.poly != null && !/^\s*\d+\s*:\s*\d+\s*$/.test(String(args.poly))) return `poly must be a ratio such as 3:2, got ${args.poly}`;
  return null;
}
