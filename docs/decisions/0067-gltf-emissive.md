# 0067: glTF emissive materials

Date: 2026-11-13. Status: accepted. Focus 4 (graphics, decision 0060).

## Context

The `Emissive` component (decision 0063) makes a whole entity glow. Models that glow in parts
(lit windows, screens, lava cracks) carry that in their glTF material, which the loader ignored.

## Decision

The loader reads `emissiveFactor`, `emissiveTexture` (as sRGB) and the
`KHR_materials_emissive_strength` extension into `MaterialData::emissive` (factor x strength)
and `MaterialData::emissive_texture`. Each material's bind group gains a small uniform with the
emissive color and the emissive texture (white when absent). The PBR shader adds
`emissive x texture` next to the per-entity `Emissive`. With strength above 1 and `Bloom`, the
glow spreads. A `Material` override uses plain textures and no glTF emissive, as it already did
for the other textures.

`examples/assets/lamp.glb` (made in code by `crates/marionette-render/examples/make_lamp.rs`,
CC0) is a street lamp with a textured emissive lantern at strength 4.

## Alternatives

- Emissive color as per-instance data: 16 more bytes per instance for every draw, though it is
  a property of the material; per-instance stays for the entity-wide `Emissive`.

## Consequences

- A material without emissive adds zero, so existing models render as before.
