# Developer Guide for Coding Agents

This guide is designed to help autonomous agents and human developers understand the `grooph` codebase, its architecture, and development workflows.

## Project Overview

**Grooph** is a flexible drummer's metronome and practice assistant. It focuses on building complex rhythmic patterns using a visual timeline.

### Key Features
- **Modular Rhythm Builder**: Supports time signatures, stickings, and complex subdivisions (tuplets, dotted notes).
- **Visual Timeline**: Renders measures and beats.
- **Platform**: Built with Rust and `eframe` (egui), supporting both native and WASM targets.

## Architecture

The project follows a Model-View separation, though currently tightly coupled within the crate.

### 1. Data Model (`src/measure`)
This is the core domain logic.
- **Measure (`src/measure.rs`)**: The container for a sequence of beats. It handles logic like `set_beat_at`, `convert_to_tuplet_at`, and ensuring the measure is "complete" (no gaps).
- **Beat (`src/measure/beat.rs`)**: Represents a single rhythmic event (Note or Rest) with a `Duration`.
- **Duration (`src/measure/duration.rs`)**: An enum (`Simple`, `Dotted`, `Tuplet`) representing the musical length.
- **Grid (`src/measure/grid.rs`)**: Manages time resolution using integer "ticks". It calculates the Least Common Multiple (LCM) of all supported durations to ensure precise timing without floating-point errors.

### 2. Application Logic (`src/app.rs`)
- **Grooph Struct**: The main entry point for the `eframe` application.
- **Update Loop**: The `update` method handles input events (keyboard shortcuts) and dispatches commands to the `Measure`.

### 3. Rendering (`src/layout`, `src/render`)
- **Layout**: Calculates the visual position of beats based on their duration and the measure's width.
- **Render**: Draws the calculated layout using `egui` primitives.

## Core Concepts & Rules

### Measures & Auto-filling
The `Measure` struct maintains a valid state. When a beat is inserted:
- It ensures the total duration matches the time signature.
- It "fills" gaps with rests automatically.
- It handles tuplet grouping (e.g., inserting a triplet beat creates a group of 3 slots).

### Tuplets
Tuplets are handled as a group.
- A beat inside a tuplet has a `tuplet_group_id`.
- Operations on tuplets often affect the entire group (e.g., dissolving a tuplet).

### Ticks
Time is measured in "ticks".
- `DEFAULT_GRID` provides the tick resolution.
- `ticks_per_whole` is the LCM of all duration denominators.

## Development Workflow

### Build & Run
- **Run**: `cargo run`
- **Test**: `cargo test`
- **WASM**: `trunk serve` (if using Trunk)

### Testing Guidelines
- **Unit Tests**: Place tests within the module (`#[cfg(test)] mod tests`) to access `pub(crate)` helpers.
- **Helpers**: Use duration shortcuts from `src/measure/duration.rs`:
    - `q()`: Quarter note
    - `e()`: Eighth note
    - `t8()`: Triplet eighth
    - `Measure::new(TimeSignature::FOUR_FOUR)`: Standard 4/4 measure.
- **Assertions**: Avoid direct `Beat` comparison if possible. Verify specific fields like `duration` and `kind`.

### Common Tasks

#### Adding a new Duration
1.  Add the variant to `NoteValue` (if needed) or `Duration` helpers.
2.  Update `COMMON_DURATIONS` in `src/measure/duration.rs` so it's included in the `Grid`.
3.  Update `human_readable` for display.

#### modifying Measure Logic
1.  Modifications to `set_beat_at` or `fill_at` in `src/measure.rs` must preserve the invariant that the measure is fully filled.
2.  Add a regression test for any edge case (especially crossing beat boundaries).

## File Structure
- `src/measure.rs`: **Start here**. Main logic.
- `src/app.rs`: UI interaction code.
- `src/layout/`: Visual calculations.
- `src/render/`: Drawing code.
