# 0130: Scene editor view: its own crate, edits through operations, an offscreen twin

Date: 2026-11-13. Status: accepted.

## Context

There was no visual way to look at a scene: `marionette run` shows the game through its own
camera, and screenshots are single images from scene cameras. The owner wants to fly around a
scene and click things to judge what was built (plan line 10, "inspector"). Questions: where
the editor lives, how it draws its interface, how clicking finds an entity, how edits reach
the scene file, and how an agent (with no screen) can see what the person sees.

## Decision

- **Crate `marionette-editor`** (core, render, glam, winit), included by the `editor` cargo
  feature of `marionette-agent` and `marionette-cli` (on by default; decision 0062). A game
  binary built without it has no editor code. The crate does not depend on the agent: the
  caller passes a `Host` (load the scene, run an operation, queue live input, play sounds).
- **No editor code in the renderer.** The overlay (panels, markers, selection box, grid) is
  added to a copy of the frame as ordinary UI draws (bitmap font) and a few extra meshes; the
  selected model gets extra emissive, a selected sprite a tint. The renderer only gained three
  small shared pieces: `Presenter` (window surface, resize, draw; the game window uses it
  too), `Renderer::model_bounds`, and `Offscreen::render_view` (render from any `View`).
- **Picking on the CPU** against model-space bounding boxes from the loaded models (oriented
  boxes in the world), sprite quads, and screen markers for entities with a place but nothing
  drawn (cameras, lights, spawn points). Hits sort nearest first, then top sprite layer; a
  second click on the same pixel selects the next thing behind. Exact triangle picking was
  rejected: it needs the CPU meshes kept after upload (memory) for little gain when cycling
  covers overlaps.
- **Edits are operation calls** (`component_set`, `entity_create`, `entity_delete` on the
  scene file), each planned with the calls that undo it (64 kept). The editor reloads the file
  after each edit, so what is shown is always what is saved. Edits are refused while the
  simulation is away from the file state (after play or step) until reset.
- **The simulation is never driven by the editor camera.** The editor camera only chooses the
  view; play mode runs fixed ticks on wall-clock time like the game window; keys go to the
  game only while it plays through its own camera (`C`).
- **Offscreen twin**: operation `editor_view` renders the same picture, lists the entities
  under given pixels and returns the selected entity's components, so an agent can check what
  `marionette edit` shows without a screen (focus 2: paths and short lists, not pixels).

## Consequences

- Inspector text uses the 5x7 bitmap font in upper case; values are compact (`x 1  y 0.5`).
  A real text renderer would be a separate change.
- Only position nudges, duplicate and delete are edits in the window; anything else is still
  an operation call. Rotation and scale handles are not done.
- Focus check: modularity (own crate and feature, no renderer branches), tokens (`editor_view`
  output is compact; image written to a path), performance (overlay is a few dozen draws; the
  inspector reads one entity per frame), graphics (unchanged scene rendering).
