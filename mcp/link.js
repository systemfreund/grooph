// The metronome link of the `show_metronome` tool: the tool's arguments are
// the app's link parameters (see crates/link and llms.txt), so the server and
// the view build the same `?v=2&ts=…` query from them. The build also serves
// this file unbundled as /embed/link.js (crates/site), which the view
// imports, so it must not import anything. With `link`, a call changes the
// metronome of that link (`resolveSettings`): the view and the server merge
// the same way.

/** The settings a metronome link carries, as the tool's arguments, plus
 * edited polyrhythm clicks (`counter`, which no argument sets). `link` is a
 * grooph URL or its query; throws with the reason if it is no metronome
 * link. */
export function linkSettings(link) {
  const text = String(link).trim();
  const params = new URLSearchParams(text.includes("?") ? text.slice(text.indexOf("?") + 1) : text);
  if (params.has("r") || ["sub", "lvl", "bars", "space", "seed"].some((key) => params.has(key))) {
    throw new Error("link is a rhythm, not a metronome: pass the link of a metronome (from its result or model context)");
  }
  if (!params.has("ts")) throw new Error("link has no time signature (ts): pass the link of a metronome");
  // Without v=2, bpm counted the meter's beat unit (crates/link).
  if (params.get("v") !== "2") throw new Error("link is an old link (without v=2): pass the link from the latest result or model context");
  const settings = {};
  for (const key of ["bpm", "ts", "groups", "subdivision", "click", "poly", "counter", "swing", "su"]) {
    if (params.has(key)) settings[key] = params.get(key);
  }
  return settings;
}

/** The settings of `args`: with `link`, that link's settings changed by
 * the other arguments, else the arguments alone. What depends on the meter
 * (groups, click, subdivision, polyrhythm) is kept only while the meter
 * stays; a click bar and a subdivision replace each other, as `poly` does
 * edited clicks. `click: "default"`, `poly: "none"` and `swing: 50` remove
 * a setting. */
export function resolveSettings(args) {
  const given = (key) => args?.[key] != null && String(args[key]).trim() !== "";
  const settings = given("link") ? linkSettings(args.link) : {};
  const meter = (ts) => String(ts ?? "4/4").replace(/\s+/g, "");
  if (given("ts") && meter(args.ts) !== meter(settings.ts)) {
    for (const key of ["groups", "click", "subdivision", "poly", "counter"]) delete settings[key];
  }
  if (given("click") || given("subdivision")) {
    delete settings.click;
    delete settings.subdivision;
  }
  if (given("poly")) delete settings.counter;
  for (const [key, value] of Object.entries(args || {})) {
    if (key !== "link" && value != null && String(value).trim() !== "") settings[key] = value;
  }
  if (String(settings.click ?? "").trim() === "default") delete settings.click;
  if (String(settings.poly ?? "").trim() === "none") delete settings.poly;
  if (settings.swing != null && Number(settings.swing) === 50) {
    delete settings.swing;
    delete settings.su;
  }
  return settings;
}

/** What `args` change in the settings of their `link`, e.g. "subdivision
 * (default) → s, click >s s s s → (default)"; `null` without a link. */
export function changes(args) {
  if (!args?.link) return null;
  const before = linkSettings(args.link);
  const after = resolveSettings(args);
  // As metronomeQuery writes them: groups and poly without spaces.
  const shown = (value, key) =>
    value == null ? "(default)" : ["groups", "poly"].includes(key) ? String(value).replace(/\s+/g, "") : String(value).trim();
  const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])];
  const changed = keys.filter((key) => shown(before[key], key) !== shown(after[key], key));
  return changed.map((key) => `${key} ${shown(before[key], key)} → ${shown(after[key], key)}`).join(", ") || "nothing";
}

/** Query (without `?`) that opens the metronome as `args` describe (see
 * `resolveSettings`), written as the app's share links are (crates/link
 * `SharedLink::to_url`). */
export function metronomeQuery(input) {
  const args = resolveSettings(input);
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
  else if (text(args?.counter)) params.push(["counter", text(args.counter)]);
  return params.map(([key, value]) => `${key}=${encode(value)}`).join("&");
}

/** Most characters of the text arguments; `show_metronome`'s schema says
 * the same (`maxLength`). */
export const MAX_LENGTH = { link: 4000, ts: 32, groups: 32, subdivision: 8, click: 1000, poly: 16 };

/** Why `args` cannot be a metronome link, or `null`. Checks that only the
 * `known` arguments are given, the numbers, and that `link` is a metronome
 * link; the notation of `click`, whether `groups` and `subdivision` fit the
 * meter and whether `poly` does are checked afterwards on the link with the
 * app's parser (`checkLink`, see link-check.js). */
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
  if (args?.link != null) {
    try {
      linkSettings(args.link);
    } catch (e) {
      return e.message;
    }
  }
  // With a link, the swing may be the link's.
  if (args?.su != null && args?.swing == null && args?.link == null) return "su needs a swing amount (swing 50-75)";
  if (args?.ts != null && String(args.ts).includes("+")) return `ts is the meter alone, such as 7/8; pass the grouping as groups, e.g. "3+2+2", got ${args.ts}`;
  if (args?.ts != null && !/^\s*\d+\/\d+\s*$/.test(String(args.ts))) return `ts must be a time signature such as 7/8, got ${args.ts}`;
  if (args?.groups != null && !/^\s*\d+(\s*\+\s*\d+)+\s*$/.test(String(args.groups))) return `groups must be beat groups such as 3+2+2, got ${args.groups}`;
  if (args?.subdivision != null && args?.click != null) return "use either subdivision or click, not both: the subdivision is a click bar of its own";
  if (args?.poly != null && String(args.poly).trim() !== "none") {
    const ratio = String(args.poly).match(/^\s*(\d+)\s*:\s*(\d+)\s*$/);
    if (!ratio) return `poly must be a ratio such as 3:2, got ${args.poly}`;
    const [n, m] = [Number(ratio[1]), Number(ratio[2])];
    const gcd = (a, b) => (b ? gcd(b, a % b) : a);
    if (n > 16 || m < 1 || n / gcd(n, m) < 2) return `poly ${n}:${m} is no polyrhythm: n is 2 to 16 and must not reduce to one click (2:4 is 1:2)`;
  }
  return null;
}
