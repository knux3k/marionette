"""Makes the Unreal-convention FBX fixture (self-made, CC0 like every test fixture here).

Unreal Engine works in centimeters with +Z up; its FBX exporter writes UnitScaleFactor 1
(1 unit = 1 cm) and a Z-up axis system. This script writes the crate from ../fbx/crate.glb the
same way, so the importer test can check that a Z-up centimeter FBX lands in meters, +Y up.

Run in a neutral folder so no machine path ends up in the file (see ../fbx/make_fixtures.py
for how to get Blender's Python module):
    cp ../fbx/crate.glb . && python make_fixtures.py
Writes crate_unreal_cm.fbx.
"""
import bpy, os
out = os.getcwd()
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=os.path.join(out, "crate.glb"))
scene = bpy.context.scene
scene.unit_settings.system = "METRIC"
scene.unit_settings.scale_length = 0.01  # one Blender unit is one centimeter
for o in scene.objects:
    if o.parent is None:
        o.location = [c * 100.0 for c in o.location]
        o.scale = [s * 100.0 for s in o.scale]
bpy.context.view_layer.update()
bpy.ops.export_scene.fbx(filepath=os.path.join(out, "crate_unreal_cm.fbx"), axis_forward="-Y",
                         axis_up="Z", apply_scale_options="FBX_SCALE_NONE", path_mode="STRIP")
print("ok")
