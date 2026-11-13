# 0064: Bloom through an HDR target, opt-in

Date: 2026-11-13. Status: accepted. Focus 4 (graphics, decision 0060).

## Context

Decision 0063 tone maps inside the forward PBR shader, so colors above 1.0 are gone by the time
anything could spread them. Lamps, emissive screens and the sun on metal looked painted on:
bright, but with no glow around them.

## Decision

Resource `Bloom { threshold, intensity }`. Absent, nothing changes. Present:

1. Models (and the sky background) draw into a floating-point target (`Rgba16Float`,
   multisampled like the final target, resolved once). The PBR shader skips tone mapping and
   clamping (flag bit `FLAG_HDR`); fog is applied in linear light with the fog color mapped back
   through the tone curve (`post::untonemap`), and so is the clear color, so fully fogged pixels
   and the background come out the same color as without bloom.
2. Glow chain (`crates/marionette-render/src/post.rs`, `shaders/post.wgsl`): a prefilter pass to
   half size keeps light above `threshold` (soft quadratic knee of half the threshold) and
   weights each tap by 1 / (1 + brightness) so single bright pixels do not flicker; then halving
   passes down to the smallest side under 16 pixels (at most six levels), then 3x3 tent passes
   back up, each added onto the next larger level.
3. The final pass starts with a full-screen composite (image + glow x intensity / levels, then
   exposure and the tone curve when `ToneMapping` is present, else a clamp), keeping the scene
   depth from step 1, then draws sprites and UI as before. Sprites and UI never glow and are not
   tone mapped.

Targets are kept per size and shared by every view of a frame (views are drawn one after the
other in one encoder), so two eyes of one size share one set; sizes not used in a frame are
freed, and a frame without `Bloom` frees them all.

## Alternatives

- Glow into a second color attachment of the existing pass: no extra full-size target, but
  sprite and UI pipelines would need the same attachments, and tone mapping before the glow is
  added clips the light that should spread.
- 13-tap downsample (as in several engines): slightly smoother; four bilinear taps (a 4x4 box)
  with the brightness weighting were enough at the proof scenes' sizes.
- Bloom inside `ToneMapping`: one resource fewer, but bloom costs memory and passes that a game
  should opt into.

## Consequences

- Memory per target size: one HDR image (8 bytes per pixel), its multisampled copy when MSAA is
  on (32 bytes per pixel at 4x), and the glow levels (about a third of the HDR image). At
  1920x1080 with 4x MSAA about 90 MB.
- Passes: one HDR pass, `2 x levels - 1` glow passes (11 at 1080p), one composite draw. Draw
  calls in `RenderStats` include the glow passes.
- Multisampled edges are resolved before tone mapping, so very bright edges against dark
  backgrounds alias slightly more than without bloom.
- Measured cost: see PROOFS.md, "More looks".
