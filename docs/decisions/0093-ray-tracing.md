# 0093: Ray-traced shadows and reflections (opt-in, with fallback)

Date: 2026-11-13. Status: accepted. Focus 4 (graphics), focus 3 (performance), focus 1.

## Context

The owner asked for ray tracing. Shadow maps miss small and far details and need six passes
per point light; reflections of other objects were missing (decision 0092 gives an
approximation). wgpu 30 has ray queries behind `Features::EXPERIMENTAL_RAY_QUERY`
(documented for Vulkan; marked experimental, "may have major bugs", API may change) and
ray tracing pipelines behind `EXPERIMENTAL_RAY_TRACING_PIPELINES`. Mesa's lavapipe (25.2)
implements `VK_KHR_ray_query`, so the cloud container can run and test it.

## Decision

Resource `RayTracing { shadows, reflections, max_roughness, sun_size_degrees }`.

- Device: `Gpu::new` asks for ray queries when the adapter has them (experimental features
  token, acceleration structure limits from the adapter); `MARIONETTE_RAY_TRACING=0` turns this
  off. Without them `RayTracing` does nothing and the scene keeps its other looks, so scenes
  should keep `Shadows`/`CastShadows`/`ReflectionProbe` as the fallback.
- Ray queries from the fragment shader, not ray tracing pipelines: they fit the forward
  renderer (no G-buffer or extra passes), and they are the less experimental of the two.
- Shader: `pbr.wgsl` has three stub functions between `RT:BEGIN` and `RT:END`; the ray tracing
  variant (`raytrace.rs`) replaces them with `shaders/raytrace.wgsl` and enables
  `wgpu_ray_query`. Its pipelines and its lights group (the plain group plus the TLAS and three
  storage buffers) are built the first time a scene asks. Without the flags set it computes the
  same as the plain shader.
- Scene: a BLAS per static mesh part (first frame it is drawn), one TLAS rebuilt each frame
  from that frame's instances (custom index = instance index). Hits read their material from
  the instance buffer itself, plus per-triangle data made on the CPU at upload: face normal,
  base color texture and emissive at the triangle's center.
- Shadows: one ray per light per pixel toward every directional, point and spot light (in
  range); shadow map passes are skipped. `sun_size_degrees > 0` spreads sun rays over the
  disc, rotated per pixel (soft edges with fine noise; no denoiser): two rays at opposite
  edges first, and only where they disagree (the penumbra) two more, so fully lit or fully
  shadowed pixels pay for two rays instead of four with the same result. Models holding a
  shadow-casting lamp do not block lamps (instance mask), as with shadow maps (decision 0065).
- Depth prepass: ray-traced frames draw every mesh depth-only first (same vertex entry points,
  an empty fragment shader with no color writes), then the lit pass with `LessEqual`, so the
  rays and shading run once per pixel instead of once per overlapping surface. Costs one more
  draw per mesh; on lavapipe the example scene went from about 67 to 60 ms at 640x360.
- Culling (decision 0080): ray-traced frames hide nothing; instances outside the views stay
  in the TLAS (drawn only into shadow maps, if any), so off-screen things still cast traced
  shadows and show in reflections. Sun shadow culling uses every cascade's volume.
- Reflections: one mirror ray on surfaces smoother than `max_roughness`, fading to the probe or
  environment reflection between half the limit and the limit. Hits are lit by every light
  (sun shadows traced again), environment or ambient, and emissive.

## Alternatives

- Ray tracing pipelines: needed for recursive effects and a denoised path tracer; more
  experimental in wgpu and would replace the forward renderer's structure. Revisit when wgpu
  stabilizes them.
- Hybrid screen-space plus ray tracing: cheaper rays, but screen-space traces differ per eye.
- Building BLASes for skinned meshes every frame: correct shadows from characters; a BLAS
  rebuild per character per frame. Left out for now (skinned models are not in the traced
  scene).

## Consequences

- Experimental: a wgpu or driver bug could crash or draw garbage on some GPUs; the env switch
  and the opt-in resource keep every other scene on the plain path.
- Reflections are mirror-sharp with flat normals and one color per triangle; rough
  reflections fade to the probe or environment rather than blurring (no denoiser yet).
- Cost and pictures on the desktop GPU: `docs/ai/proofs/render.md` ("Remaining lighting").
