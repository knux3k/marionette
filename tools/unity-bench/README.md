# Realistic benchmark in Unity (decision 0284)

These scripts build the scene that `marionette op perf_realistic_scene` writes as a Unity
scene and Windows player, so both engines can be timed on the same machine with the same
meshes, textures, materials, instances, motion, camera, sun and shadow settings.

## Steps

1. Generate the scene once (about 30 s, 190 MB):

   ```bash
   cargo build --profile dist -p marionette-cli
   target/dist/marionette op perf_realistic_scene '{"out":"bench/realistic.scene.json"}'
   ```

2. Time Marionette (the first run encodes the 99 textures to BC7 and caches them; time the
   second run):

   ```bash
   target/dist/marionette bench bench/realistic.scene.json --frames 1000 --json
   target/dist/marionette bench bench/realistic.scene.json --headset --frames 600 --json
   ```

3. Make an empty Unity project with the Universal Render Pipeline (Unity Hub's URP template,
   or an empty project with the `com.unity.render-pipelines.universal` package). Never use a
   game's own project.
4. Copy this folder (`Editor/`, `Runtime/`) into the project as `Assets/MarionetteBench/`.
5. Build the scene and a player without opening a window:

   ```text
   Unity -batchmode -quit -projectPath <project> -logFile build.log
         -executeMethod MarionetteBench.RealisticBenchBuilder.Build
         -benchFolder <repo>/bench/realistic -benchPlayer <out>/RealisticBench.exe
   ```

   Add `-benchStatic` for a second player with Unity's static batching on the static models.

6. Run the player (a 1920x1080 window; it quits by itself):

   ```text
   <out>/RealisticBench.exe -screen-width 1920 -screen-height 1080 -screen-fullscreen 0
         -benchOut <out>/unity.json -benchFrames 1000 -benchWarmup 120
   ```

## What matches and what does not

| | Marionette | Unity |
| --- | --- | --- |
| Meshes | `models/*.glb` | `meshes/*.bin`, the same vertices and triangles, X mirrored for Unity's axes |
| Textures | 99 PNGs, 2048x2048, mipmaps, BC7 (`TextureCompression`), 16x anisotropic | the same PNGs imported as BC7, mipmaps, 16x anisotropic |
| Materials | 257 glTF metal-roughness | 257 URP Lit, metal-smoothness map from the same data |
| Motion | `Follow` + `Velocity` systems at 60 Hz | `RealisticBenchMovers`, the same formula in `FixedUpdate` at 60 Hz |
| Lighting | sun, 4 shadow cascades to 150 m, 2048 maps, hemisphere ambient, ACES tone mapping | sun, 4 cascades to 150 m, 2048 maps, trilight ambient, URP default tone mapping |
| Output | offscreen, no present | a window, presented; vertical sync off |

Brightness and tone are close but not identical; the comparison is about cost at the same
work, not about matching pixels.
