# grooph — a flexible drummer’s metronome

groops aims to be a highly flexible, drummer‑focused metronome and practice assistant. It combines a visual timeline with nuanced rhythmic building blocks (subdivisions, tuplets, stickings, rests) so you can design click tracks and exercises that match real‑world drumming needs.

## Features (planned)

- Modular rhythm builder
  - Time signatures (e.g., 4/4, 3/4, 6/8)
  - Notes, rests, and stickings (R/L) per beat
  - Common and odd subdivisions: 1/4, 1/8, triplets, 1/16, quintuplets, sextuplets, septuplets, 1/32, nonuplets
- Audio engine
  - Low‑latency click with different voices (rim, hat, bell, etc.) (roadmap)
  - Polyrhythms and layered meters (roadmap)
- Practice flows
  - Tempo ramps, cycles, gaps, and “Time‑Feel” exercises (roadmap)
  - Presets for rudiments, linear patterns, coordination drills (roadmap)

## Getting started

### Prerequisites

- Rust toolchain (stable). Install via https://rustup.rs
- System dependencies for native eframe/egui apps (platform‑specific windowing deps).

### Build & run

```bash
# From project root
cargo run
```

If compilation fails with a Bravura font error, see Fonts section below.

### Run tests

```bash
cargo test
```

## Roadmap

- Add audio back‑end with precise scheduling and low jitter.
- Visual accents, subdivision markers, and per‑beat sticking overlays.
- Pattern editor: build, save, and share exercises (JSON/TOML files).
- Tempo automation: ramps, cycles, and gap‑clicks.
- Polyrhythm layers and mixed meters.
- Packaging: binaries for Linux/macOS/Windows.

