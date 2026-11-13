# 0190: Headsets through OpenXR

Date: 2026-11-13. Status: accepted.

## Context

VR was only emulated (`bench --headset`, stability `views: 2`): no headset had ever shown an
engine frame. The owner's target is a PC VR factory game on SteamVR and a standalone ARM64
headset, both reached through OpenXR. VR stays optional (RULES.md 6), the simulation must stay
deterministic and replayable, and the renderer and its shader are shared with other work.

## Decision

- **Two crates, split at determinism.**
  - `marionette-xr` (always in the standard engine; no headset code, pure `f32` maths and
    `libm`): `XrRig` (the play space in the world), `XrTracked` (head, grips, aim rays follow
    poses), `XrHand` + `Grabbable` + `Held` (grab and throw), `XrLocomotion` (stick movement,
    snap or smooth turn about the head, teleport by ray), `XrHandMotion` (throw velocity from
    the last 5 ticks of grip positions). Outputs that are not state: `XrHaptics` (bounded,
    32), `XrComfort` (vignette strength).
  - `marionette-openxr` (CLI feature `xr`, on by default): the runtime. OpenXR loader and
    Vulkan loader are opened at run time (`openxr` feature `loaded`, `ash` `loaded`), so it
    builds and runs everywhere and `marionette xr --probe` reports what is missing.
- **Poses are input.** Each tracked pose is eight directly set actions (`xr.head.px` ..
  `xr.head.qw`, `xr.head.tracked`; sources `head`, `left`, `right`, `left_aim`, `right_aim`),
  quantized to 0.1 mm and 1e-5 before feeding. They pass through the input queue, the
  recording and the replay like any action, and `ActionState` puts them in the state hash. A
  headset run recorded with `--record` replays headless bit for bit; tests drive XR with
  scripted poses and no runtime.
- **Controllers are fixed engine inputs.** `xr:<left|right>/<input>` with inputs
  `trigger squeeze thumbstick_x thumbstick_y thumbstick_click primary secondary menu pinch`
  (`marionette_input::XR_INPUTS`, checked when an action map loads). The runtime crate binds
  them for the simple, Touch, Index, Vive and Windows Mixed Reality controller profiles;
  runtimes that let players rebind (SteamVR) map anything else onto these. Hand tracking
  (`XR_EXT_hand_tracking`, when offered): `pinch` from thumb-to-index distance, the palm as
  grip and aim when no controller is tracked, all 26 joints to `XrHooks::hands` for drawing.
- **Rendering: Vulkan through wgpu-hal.** The runtime creates the Vulkan instance and device
  (`XR_KHR_vulkan_enable2`) with the extensions and features wgpu asks for; wgpu wraps them
  (`Instance::from_hal`, `create_device_from_hal`) and each swapchain image
  (`create_texture_from_hal`, runtime-owned memory). One sRGB swapchain per eye at the
  recommended size (`--scale`), MSAA and depth per eye, both eyes recorded by the existing
  `Renderer::render_views` into one submit. Eye poses come from `xrLocateViews` at the
  predicted display time, placed by the camera's rig, with asymmetric projections; the
  renderer is unchanged.
- **Time.** The tick stays fixed. Each frame waits on the runtime, samples input for the
  predicted display time, then runs as many ticks as display time advanced (at most 8).
  Gameplay sees the head at tick rate; the eyes use the fresher predicted pose (less
  latency, no judder from the 60 Hz tick).
- **Comfort basics.** Snap turn (30 degrees default, hysteresis on the stick), smooth turn,
  head- or hand-relative stick movement, teleport (ray, then a drop if it hits a wall; slopes
  up to about 45 degrees), a vignette drawn over each eye by a separate small pass while
  moving or turning smoothly (the renderer and PBR shader are untouched). Recentering and
  the guardian are the runtime's (SteamVR, standalone headsets).
- **Physics change (generic).** A dynamic body whose `Velocity` or `AngularVelocity` was set by
  gameplay since the last write-back takes that velocity (same bit-compare rule as teleports).
  Held dynamic objects follow the hand by velocity, so they still collide; releasing keeps the
  hand's velocity (a throw).
- **Audio.** `XrHooks::head` passes the rendered head pose to `Player::set_head` (decision
  0075) every frame.

## Alternatives

- **Poses as a separate per-tick resource fed outside the input queue**: needs its own
  recording and replay path; rejected, the input path already records, replays and hashes.
- **Throw velocity from the runtime (`xrLocateSpace` velocities)**: better filtered, but not
  part of the recording, so replays would differ; history over 5 ticks is deterministic and
  close to common engines' default throw smoothing.
- **One array swapchain with multiview (one pass for both eyes)**: halves draw submission on
  the CPU; needs multiview pipelines in the renderer and shader (contended files). Kept for a
  follow-up: the per-view renderer already accepts any number of targets.
- **Depth submitted for reprojection (`XR_KHR_composition_layer_depth`)**: helps runtimes
  reproject missed frames; follow-up.
- **D3D12 on Windows**: SteamVR supports Vulkan on Windows and Linux alike; one path is
  simpler to keep right.

## Consequences

- `cargo test -p marionette-cli --test xr`: grab and throw, replay, snap turn, teleport, stick
  movement with vignette; with `MARIONETTE_XR_RUNTIME=1` it also runs the example on a real
  runtime (CI: Monado's simulated headset under Xvfb on lavapipe).
- Recordings of headset play are larger (poses change every tick).
- Shipping a game: the OpenXR loader (`openxr_loader.dll` on Windows, Apache-2.0) goes next to
  the executable; Linux uses the system `libopenxr_loader.so.1`.
- Not verified until a person wears a headset: comfort, controller bindings on each device,
  hand tracking, timings on a real GPU.
