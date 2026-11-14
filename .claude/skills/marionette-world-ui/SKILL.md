---
name: marionette-world-ui
description: Put panels, buttons, sliders, lists and text into the 3D world (VR menus, wrist panels) with `UiPanel` and `UiContent`, and drive them from pointers or touch. Use when UI must live in the scene instead of on the screen, or when a controller laser or finger should press it.
---

# World-space UI

Opt-in: add the cargo feature `ui3d` (or `Engine::builder().module(marionette_ui3d::Ui3dModule)`).
Games that do not use it carry none of it. The 2D screen UI (`UiNode`) stays for HUDs.

## A panel is one entity

`UiPanel` (size in meters, `columns`, `padding`, `text_size`, colors, `face`, `max_distance`,
`interactive`) plus `UiContent` (a list of UiItem: `id`, `span`, `height`, `widget`). Widgets:
Label, Button (sets input `action`), Toggle, Slider, Progress, List, Spacer.
Items flow left to right across the columns, then down. Values (slider, toggle, list
selection, scroll) live in the item, so they are saved, hashed and replayed.

Place it with `Transform` (centre, front faces local +Z). `face`: Fixed, Yaw (upright,
turns toward the viewer) or `Camera`. `UiAnchor` follows another entity by SceneRef
(`relative` to its axes, `smoothing` seconds of lag), e.g. a panel on a wrist or in front of
the head.

## Size

`text_size` is the letter height in meters. 0.03 to 0.04 reads comfortably at 0.6 m. The text
is a distance field, so it stays sharp at any size, distance and angle; at about 1 pixel per
letter it is unreadable whatever the renderer does, so check the size against distance.

## Fonts

Without setup the small built-in font is used (capitals only). For a real font set the
resource `UiFontConfig` `font` to a TrueType or OpenType file relative to the scene (its
license is yours to check). Characters outside Latin-1 and a few symbols show as `?`.

## Input: the pointer hook

Any source writes UiPointers each tick: a ray (`origin`, `direction`, `pressed`, `scroll`)
or a touch (fingertip `origin`; pushing within 1 cm of the panel presses). The module hits
the nearest interactive panel's front, changes widget values and fills UiEvents
(Click, Toggle, Slide, Select, Scroll) for the tick. A slider keeps following a held
pointer after it leaves the panel. Nothing depends on a headset: a mouse ray from the camera,
a script or an operation works the same.

## Check without a screen

- `marionette op ui3d_panels '{"session":"s1"}'` lists items and their rectangles.
- `ui3d_point` sets pointers, steps, and returns the events: press in one call, release in
  the next to click.
- `marionette screenshot` shows the panel; `overflow: true` in `ui3d_panels` means content
  below the panel edge is not drawn: make the panel taller or use a List.

## Traps

- Seen from behind, a panel is not drawn and not hit. Use `face` or turn it.
- Hundreds of panels are cheap when unchanged (layout and mesh are cached); a changing value
  rebuilds only that panel.
