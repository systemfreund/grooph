// The metronome link of the `show_metronome` tool: the tool's arguments are
// the app's link parameters (see crates/link and llms.txt), so the server and
// the view build the same `?v=2&ts=…` query from them. The build also serves
// this file unbundled as /embed/link.js (crates/site), which the view
// imports, so it must not import anything.

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
  if (text(args?.groups)) params.push(["groups", text(args.groups).replace(/\s+/g, "")]);
  if (text(args?.swing)) params.push(["swing", text(args.swing)]);
  // Eighths swing by default: the app writes su only for sixteenths.
  if (text(args?.su) && text(args.su) !== "8") params.push(["su", text(args.su)]);
  if (text(args?.click)) params.push(["click", text(args.click)]);
  if (text(args?.subdivision)) params.push(["subdivision", text(args.subdivision)]);
  if (text(args?.poly)) params.push(["poly", text(args.poly).replace(/\s+/g, "")]);
  return params.map(([key, value]) => `${key}=${encode(value)}`).join("&");
}

/** Most characters of the text arguments; `show_metronome`'s schema says
 * the same (`maxLength`). */
export const MAX_LENGTH = { ts: 32, groups: 32, subdivision: 8, click: 1000, poly: 16 };

/** Why `args` cannot be a metronome link, or `null`. Checks that only the
 * `known` arguments are given and the numbers; the notation of `click`,
 * whether `groups` and `subdivision` fit the meter and whether `poly` does
 * are checked
 * afterwards on the link with the app's parser (`checkLink`, see
 * link-check.js). */
export function argumentError(args, known) {
  const unknown = Object.keys(args || {}).filter((key) => !known.includes(key));
  // Far beyond any bar the app writes, short enough to check at once.
  for (const [key, max] of Object.entries(MAX_LENGTH)) {
    if (args?.[key] != null && String(args[key]).length > max) return `${key} is too long (at most ${max} characters)`;
  }
  if (unknown.includes("counter")) return "counter is not an argument: pass the polyrhythm as poly, e.g. \"3:2\"";
  if (unknown.length) return `unknown argument ${unknown.join(", ")}; the arguments are ${known.join(", ")}`;
  const whole = (value, min, max) => Number.isInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
  if (args?.bpm != null && !whole(args.bpm, 20, 300)) return `bpm must be a whole number from 20 to 300, got ${args.bpm}`;
  if (args?.swing != null && !whole(args.swing, 50, 75)) return `swing must be a whole number from 50 to 75, got ${args.swing}`;
  if (args?.su != null && !["8", "16"].includes(String(args.su))) return `su must be 8 or 16, got ${args.su}`;
  if (args?.su != null && args?.swing == null) return "su needs a swing amount (swing 50-75)";
  if (args?.ts != null && String(args.ts).includes("+")) return `ts is the meter alone, such as 7/8; pass the grouping as groups, e.g. "3+2+2", got ${args.ts}`;
  if (args?.ts != null && !/^\s*\d+\/\d+\s*$/.test(String(args.ts))) return `ts must be a time signature such as 7/8, got ${args.ts}`;
  if (args?.groups != null && !/^\s*\d+(\s*\+\s*\d+)+\s*$/.test(String(args.groups))) return `groups must be beat groups such as 3+2+2, got ${args.groups}`;
  if (args?.subdivision != null && args?.click != null) return "use either subdivision or click, not both: the subdivision is a click bar of its own";
  if (args?.poly != null) {
    const ratio = String(args.poly).match(/^\s*(\d+)\s*:\s*(\d+)\s*$/);
    if (!ratio) return `poly must be a ratio such as 3:2, got ${args.poly}`;
    const [n, m] = [Number(ratio[1]), Number(ratio[2])];
    const gcd = (a, b) => (b ? gcd(b, a % b) : a);
    if (n > 16 || m < 1 || n / gcd(n, m) < 2) return `poly ${n}:${m} is no polyrhythm: n is 2 to 16 and must not reduce to one click (2:4 is 1:2)`;
  }
  return null;
}
