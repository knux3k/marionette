# Proofs: scene editor view

Decision 0130.

- `cargo test -p marionette-editor`: fly camera starts exactly at a scene camera's view, flies,
  turns (pitch clamped), frames a box, pans and zooms in 2D; screen rays and projection;
  nearest box first, rotated boxes, markers within 10 px, top sprite first at equal distance;
  inspector values read through the registry and printed compactly, wrapped and cut; overlay
  adds only top-layer UI, the highlight and the grid; edit plans and their undo calls.
- `cargo test -p marionette-cli --test editor`: through a real agent on a temp project, clicks
  select a model, a list row and a light marker; move, duplicate and delete write the scene
  file through the operations and undo restores it; edits wait for a reset after a step.
  `editor_view` on `examples/showcase.scene.json` (GPU): the pick at (640, 400) hits the gem,
  then the ground; inspector components come back; read-only output is refused outside
  `.marionette/`; framing the crate puts it under the image center.
- Window, Linux container, Mesa lavapipe, virtual X display (Xvfb), driven with `xdotool`:
  `marionette edit examples/showcase.scene.json` opens; a click selects the gem; arrow keys
  move it in the file (+0.25 m each); Ctrl+D copies it as #7 and Ctrl+Z removes the copy;
  right-drag turns; P plays, C shows the game camera; `--frames 120` closes by itself; a
  window destroyed from outside closes cleanly (the game window too: a lost surface now ends
  the loop instead of failing in `Surface::configure`).
