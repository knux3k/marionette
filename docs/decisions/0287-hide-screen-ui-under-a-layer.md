# 0287: Hide screen UI under a layer

Date: 2026-11-16. Status: accepted. Focus 1 (modularity).

## Context

A photo mode (or a cutscene) hides the game's interface while keeping its own overlay. Each
game module would otherwise have to know about the photo mode and hide its own nodes.

## Decision

`marionette_core::ui::ScreenUiCutoff { on, min_layer }` (a resource): while `on`, screen UI
nodes whose `layer` is below `min_layer` are not drawn and take no mouse clicks. Nodes on world
canvases are not affected. Without the resource nothing changes.

## Rejected

- A per-node hidden flag: every module would have to set it on its own nodes and remember
  which ones it turned off.
