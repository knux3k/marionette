# Status by feature: slice, usable, proven

"Done" in a plan says a part landed with tests. It does not say how far a game can lean on
it. This table does (decision 0274). Read it when choosing what to build on or what to work
on next. Update a row when work moves it; every level needs the evidence named in its row.

| Level | Meaning |
| --- | --- |
| slice | Works on the paths its own tests take. A real game meets a missing piece soon. |
| usable | A game can be built on it today. The gaps are known and listed. |
| proven | Usable, and a real game, real content or real hardware has run it, beyond its own tests. |

What is still unchecked on hardware or real content is in [STATE.md](STATE.md), "UNVERIFIED".
Proof commands are in [PROOFS.md](PROOFS.md).

## Simulation and scenes

| Feature | Level | Evidence | Next level needs |
| --- | --- | --- | --- |
| World, scene format, validation, determinism | proven | Six proof games; a real factory game's integer simulation ported and matched state for state (0072) | - |
| Save games (0076) | usable | Round trips, damaged files, off-tick writes with budgets | A real game's saves across versions |
| Scene edits at size (0270) | usable | 10,000 entity budgets | Single live edits that do not rebuild the world |
| Parent and child transforms (0272) | slice | Unit, physics and operation tests | A game that uses it; listing children; script detach; importers keeping hierarchy |
| Jobs in Rust (0024) | proven | The ported factory simulation runs on it | - |
| Job TypeScript (0030, 0070) | usable | Operator-level match with the interpreter (0273), swarm example | A game's hot loop written in it; ARM and desktop timings |
| Scripts (0022, 0271) | usable | Six proof games; budgets for 1,000 scripts | Thousands of scripted entities (not the goal: use jobs); allocation counts |
| Navigation and round worlds (0230 to 0232) | usable | Crowd and planet tests, stability modes | A game with real levels |

## Rendering

| Feature | Level | Evidence | Next level needs |
| --- | --- | --- | --- |
| Opaque lit scenes: PBR, shadows, sky, HDR, bloom, fog (0063 to 0067, 0090 to 0092) | usable | Reference images, pixel tests of material textures (0273), one desktop GPU offscreen | AMD and Intel GPUs; a real game's content |
| Mipmaps, anisotropy, cut-out and see-through materials, particles, BC7 textures, one-pass stereo (0280 to 0285) | slice | Tests on lavapipe only | A hardware GPU; a game's content; one-pass stereo in the OpenXR runtime |
| Culling, level of detail, render scale, foveation (0080 to 0082) | usable | Budgets and stability on one desktop GPU | A headset |
| Ray tracing (0093) | slice | Lavapipe and one desktop GPU; skinned models not traced | A scene that needs it, on more than one GPU |
| Skeletal animation (0028, 0074) | slice | Numeric pose tests on a generated puppet | A real rigged character on a hardware GPU |
| Game meshes and shaders (0180, 0181) | slice | Lavapipe only | A hardware GPU and a game using them |
| Sprites and screen UI | usable | 2D proof games | A game with a large interface; text input |
| World-space UI (0200, 0201) | slice | Offscreen only | A headset, a TrueType font, touch |

## Play

| Feature | Level | Evidence | Next level needs |
| --- | --- | --- | --- |
| Physics (Rapier) | usable | Proof games; behaviour tests for gravity, bounce, friction, sensors (0273) | Joints; a game with stacking or vehicles |
| Input, windowed play (0170 to 0173) | usable | Checked on a real screen | Gamepads on hardware, Linux and macOS screens |
| Audio: files, 3D sound, music layers (0075) | slice | Numbers tested; never listened to | A person listening on speakers and a headset |
| Synth and streamed audio (0220) | slice | Rendered to numbers; one device run | Listening; OGG |
| Headsets through OpenXR, hands, grab (0190) | slice | Simulated headset and scripted poses only | A worn headset at 90 Hz |
| Networking, Steam (0210 to 0213) | slice | Loopback only | Two machines; a Steam client |

## Content

| Feature | Level | Evidence | Next level needs |
| --- | --- | --- | --- |
| Generation: images and models (0045 to 0048) | proven | 19 models made through live providers and used by the proof games | - |
| CC0 pool (0041, 0071) | usable | 207 assets, checked in CI | A live search and download through the online adapters |
| Unity importer and prefabs (0027, 0040) | slice | Hand-written prefab files; one read-only dry run on a real project | A real pack imported and looked at |
| Unreal importer (0029) | slice | Hand-written T3D and synthetic packages | Any real Unreal project |
| FBX to glTF | slice | Files from two exporters | Files from the common authoring tools |
| Porting tools (0072, 0240) | usable | One real Unity slice scanned, scaffolded and ported | An Unreal project; API tables checked against current documentation |

## Tools and shipping

| Feature | Level | Evidence | Next level needs |
| --- | --- | --- | --- |
| Operations, CLI, MCP (0061, 0062, 0290) | usable | Every proof game was built through them | A session of a real MCP client recorded |
| Skills and handoffs (0160, 0161) | usable | Installed into games, checked by tests | Several real agents on one game |
| Realistic render benchmark with a Unity twin (0284) | usable | Ran beside Unity on an RTX 2080: Marionette 19.3 ms, Unity 11.1 ms per frame (proofs/performance.md) | Marionette as fast as Unity on it |
| Stability and performance harness (0023, 0260) | proven | All fast modes within budget on a hardware GPU; runs in CI | The long soak on a hardware GPU |
| Editor (0130) | slice | Offscreen view and edit tests | A person using the window on a real scene |
| Game starter (0150) | slice | Made games pass their tests; not played | A person playing one |
| Packaging (0140) | slice | Built in CI; binaries ran headless on generic ARM64 | A run on a Deck, a Frame or a clean desktop |
