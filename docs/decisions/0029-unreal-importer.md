# 0029: The Unreal importer reads exports, not packages

Date: 2026-11-12. Status: accepted.

## Context

Workstream W6 brings assets a person owns from Unreal projects, Marketplace/Fab downloads and
the launcher's vault cache into the owned library, under the same license rules as the Unity
importer (decision 0018). Unreal keeps its assets in `.uasset`/`.umap` packages: a binary,
undocumented format that changes with every engine version (versioned property tags, custom
serialization per class, editor-only bulk data, optional Zen/IoStore containers in cooked
builds, often encrypted). What a person can export from the editor is plain: FBX for static
and skeletal meshes and animations, glTF through Epic's glTF Exporter plugin, PNG/TGA/BMP/EXR
for textures, and "Unreal object text" (`.T3D`/`.COPY`) for any object, including materials.
Fab also offers some products as plain FBX/glTF/OBJ downloads.

## Decision

- **Read exports, diagnose packages.** `crates/marionette-import/src/unreal.rs`. The importer
  takes an Unreal project folder (`*.uproject`), a plugin folder (`*.uplugin`) or a bare
  content folder (`Content/` holding packages, as in the vault cache and Fab downloads) and
  imports the plain files in it through the existing texture and model paths (FBX converted to
  glTF by decision 0027). Packages are not parsed. Each one is listed as skipped
  (`import.unreal_package`) with what can be told without parsing: whether it starts with the
  package tag, its asset class (found as a length-prefixed name in the name table, in a fixed
  priority so a level or Blueprint is not reported as the meshes it references), the file name
  (never the folders, which can name the author's machine) of the source it was imported from
  (`RelativeFilename` in its import data), and the exact export step for that class. A `.uexp`
  beside it marks it as cooked. Cooked containers and their parts (`.pak`, `.utoc`, `.ucas`,
  `.uexp`, `.ubulk`, ...) are skipped as `import.unreal_cooked`. Code (`.h`, `.cpp`, `.cs`,
  descriptors, `.ini`) is skipped with a pointer to `port_scan`. `Binaries/`, `Intermediate/`,
  `Saved/`, `DerivedDataCache/` and `Build/` are not walked.
- **Engine content** (rule 7, no override): anything under `Engine/`; folders of Epic's
  templates, Starter Content and mannequins (`StarterContent`, `ThirdPerson[BP]`,
  `FirstPerson[BP]`, `TopDown[BP]`, `SideScroller[BP|2D]`, `VehicleTemplate`,
  `LevelPrototyping`, `Mannequin(s)`, `AnimStarterPack`, `FeaturePacks`); every file of a
  plugin whose `.uplugin` says `CreatedBy` Epic; and, as for Unity, a declaration naming Epic as
  publisher. Epic's free sample packs (published by Epic) are refused through the declaration's
  publisher; their folder names are too varied to list.
- **Materials from object text.** A `.T3D`/`.COPY` export holding a `MaterialInstanceConstant`
  is mapped by parameter names (texture, vector and scalar parameters such as BaseColor,
  Normal, ORM/ARM, AO, Emissive, Tint, Roughness, Metallic, Normal Strength, Emissive
  Intensity, Tiling), plus blend mode, mask clip value and two-sidedness from
  `BasePropertyOverrides`. A `Material` export is mapped from the nodes directly connected to
  its inputs (texture samples and texture parameters with the output channel used, constants,
  vector and scalar parameters); anything computed by other nodes is named in the notes. A
  texture whose G channel feeds roughness and B feeds metallic is the glTF packing already and
  is referenced as it is (new `MetalLayout::Gltf`). Unreal colors are linear, so no sRGB
  conversion. Unreal normal maps are DirectX style; the material gets a green-flipped
  `normal.png`. Texture references (`/Game/Props/T_Crate_D`) resolve to library ids by the
  object path each imported texture records (`source.unreal_path`; `Content/` mounts at `/Game`,
  a plugin's at `/<Plugin>`), this import first, then earlier imports, then a unique texture
  file of the same name in this import (exports often sit outside `Content/`). The Unity and
  Unreal material paths share one writer (`material_from` in `src/import.rs`); the Unity output
  is unchanged.
- **Coordinates.** Unreal is +Z up in centimeters; its FBX exports state that in
  GlobalSettings, which the FBX converter already honors (test
  `unreal_convention_fbx_lands_in_meters_y_up`). glTF exports are Y up in meters by the glTF
  specification. Placement data in levels (where `port_convert` would apply) is not imported.
- **BMP** textures are now converted to PNG like TGA and JPEG (Unreal exports BMP).

## Alternatives considered

- Parsing `.uasset` (as community tools such as CUE4Parse or UAssetAPI do): large, tied to
  engine versions, needs per-class code for meshes and textures, and cooked data is often
  encrypted. It would also read content whose license was never meant to leave the engine more
  easily than the license rules can judge it. Exports are what a person who owns the asset can
  make in a minute, and the report says which export to make.
- Running the Unreal Editor headless with a Python export script: needs the editor (tens of
  gigabytes) on the user's machine and opens their project; possible later as an optional
  helper, never a requirement.
- Mapping material graphs fully: a graph is a program; direct connections cover the common
  material instances and simple materials, and the rest is listed for a person or an agent.

## Consequences

- An Unreal project yields models, textures and materials once the person has exported them;
  the import report lists every package with the export step for its class.
- Untested on real Unreal output: the T3D layout is written from knowledge of the editor's
  format (fixtures are hand-written), the package class and `RelativeFilename` peek is tested on
  synthetic bytes only, and FBX from Unreal's own exporter is untested (the fixture is Blender
  writing Unreal's convention).
