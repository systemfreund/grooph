### Short answer
We compare only `n` and `base` because they define the tuplet “grid” (the quantization family). The `m` value is the length of a particular note inside that grid (how many grid units it spans). Requiring `m` to match would incorrectly forbid many valid, grid‑aligned tuplet notes and rests.

### What `n`, `m`, and `base` mean
- `base`: the underlying note value the tuplet subdivides (e.g., `Eighth`).
- `n`: how many equal parts the base beat is subdivided into (the tuplet ratio’s “in the time of n”). For triplets this is `3`.
- `m`: how many of those tuplet parts the specific duration occupies. It’s not the identity of the grid; it’s the span within that grid.

In code terms, the tick size for a `Tuplet { n, m, base }` is
```
m / (n * base.denominator()) of a whole note
```
The grid identity is determined by `(n, base)`; `m` just tells you how many grid units.

### Why only `(n, base)` in the early guard
The early guard’s responsibility is to prevent mixing incompatible grids (e.g., sticking a simple eighth into a triplet‑eighth position). For that purpose:
- Same `(n, base)` ⇒ same tuplet family/grid.
- Different `(n, base)` or non‑tuplet ⇒ different grid → reject.

If we also required `m` to be equal, we’d block perfectly valid edits such as:
- Replacing a one‑unit tuplet rest with a two‑unit tuplet note (still aligned to the same grid).
- Shortening/lengthening within the same tuplet group, as long as alignment, remaining space, and exact re‑spelling are satisfied.

Those operations are musically fine; the only non‑negotiable rule is “stay in the same tuplet grid.” The exact span (`m`) is constrained elsewhere by:
- Tick arithmetic (it must fit without overflowing the group/measure), and
- Remainder spelling rules (must be exactly fillable by allowed durations).

### Examples
- Triplet eighth grid: `(n=3, base=Eighth)`
  - `Tuplet {3, 2, Eighth}` (your `t8()`) spans two grid units and is allowed.
  - `Tuplet {3, 4, Eighth}` spans four grid units. It’s the same grid; whether it’s allowed depends on space and catalog spelling, not the guard. We generally don’t auto‑produce non‑catalog values like `m=4`, but a user‑requested one could be accepted if it fits and you choose to support it.
  - A simple `e()` is not in the grid (no tuplet) → reject by the guard.

### Where `m` does matter (but not in the guard)
- Fit and alignment: `new_ticks` must be an integer multiple of the grid unit and must not cross group/measure boundaries unless you have explicit logic to allow it.
- Remainder spelling: when a rest is partially consumed, the remainder must be expressible using your allowed durations (typically from `COMMON_DURATIONS`). This prevents producing odd `m` spellings automatically.

These checks happen in the shrink/growth logic, not in the early guard. The guard is intentionally minimal: only assert grid compatibility.

### TL;DR
- `(n, base)` identifies the tuplet grid; `m` is how many grid units this particular duration spans.
- The early guard uses `(n, base)` to prevent mixing grids.
- Enforcing `m` equality would over‑restrict valid edits (e.g., longer/shorter notes within the same tuplet group). Let the subsequent tick/fit and remainder‑spelling logic handle whether a given `m` is acceptable in context.