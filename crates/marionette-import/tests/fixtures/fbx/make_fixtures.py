"""Builds the FBX test fixtures (all made for the project; CC0).

Run inside this folder with Blender's Python module (Collecting bpy
  Using cached bpy-5.3.0-cp313-cp313-manylinux_2_28_x86_64.whl.metadata (2.0 kB)
Collecting cattrs (from bpy)
  Using cached cattrs-26.2.1-py3-none-any.whl.metadata (8.5 kB)
Collecting cython (from bpy)
  Using cached cython-3.3.1-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl.metadata (13 kB)
Requirement already satisfied: numpy<3.0,>=2.2 in /usr/local/lib/python3.13/site-packages (from bpy) (2.5.3)
Requirement already satisfied: requests in /root/.local/lib/python3.13/site-packages (from bpy) (2.34.2)
Collecting zstandard (from bpy)
  Using cached zstandard-0.25.0-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.whl.metadata (3.3 kB)
Requirement already satisfied: attrs>=25.4.0 in /usr/local/lib/python3.13/site-packages (from cattrs->bpy) (26.1.0)
Requirement already satisfied: typing-extensions>=4.14.0 in /usr/local/lib/python3.13/site-packages (from cattrs->bpy) (4.16.0)
Requirement already satisfied: charset_normalizer<4,>=2 in /root/.local/lib/python3.13/site-packages (from requests->bpy) (3.5.1)
Requirement already satisfied: idna<4,>=2.5 in /root/.local/lib/python3.13/site-packages (from requests->bpy) (3.20)
Requirement already satisfied: urllib3<3,>=1.26 in /root/.local/lib/python3.13/site-packages (from requests->bpy) (2.8.0)
Requirement already satisfied: certifi>=2023.5.7 in /root/.local/lib/python3.13/site-packages (from requests->bpy) (2026.10.18)
Using cached bpy-5.3.0-cp313-cp313-manylinux_2_28_x86_64.whl (427.5 MB)
Using cached cattrs-26.2.1-py3-none-any.whl (74 kB)
Using cached cython-3.3.1-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl (3.4 MB)
Using cached zstandard-0.25.0-cp313-cp313-manylinux2014_x86_64.manylinux_2_17_x86_64.whl (5.5 MB)
Installing collected packages: zstandard, cython, cattrs, bpy
Successfully installed bpy-5.3.0 cattrs-26.2.1 cython-3.3.1 zstandard-0.25.0):
    python make_fixtures.py
Writes crate.fbx, crate_embedded.fbx, crate_zup_units.fbx, bend.fbx, checker.png and Blender's own
glTF exports crate.glb and bend.glb, which the tests use as the reference. Then, with the Assimp
command line tool (a second, independent FBX writer that writes FBX 7.5):
    assimp export crate.glb crate_assimp.fbx -f fbx
and Blender's import of that file as its reference:
    python -c "import bpy; bpy.ops.wm.read_factory_settings(use_empty=True); \
      bpy.ops.import_scene.fbx(filepath='crate_assimp.fbx'); \
      bpy.ops.export_scene.gltf(filepath='crate_assimp_blender.glb', export_format='GLB')"
"""
import bpy, math, os, struct, zlib
out = os.getcwd()

def png(path, w, h, fn):
    raw = b"".join(b"\x00" + bytes(sum((fn(x, y) for x in range(w)), ())) for y in range(h))
    def chunk(t, d): return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    open(path, "wb").write(data)

png("checker.png", 8, 8, lambda x, y: (255, 64, 32, 255) if (x // 2 + y // 2) % 2 else (32, 64, 255, 255))

def reset():
    bpy.ops.wm.read_factory_settings(use_empty=True)

def textured_material(name):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    bsdf = nt.nodes.get("Principled BSDF")
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(os.path.join(out, "checker.png"))
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    bsdf.inputs["Roughness"].default_value = 0.5
    return m

def plain_material(name, rgba):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = rgba
    return m

def export(name, **kw):
    bpy.ops.export_scene.fbx(filepath=os.path.join(out, name), **kw)
    bpy.ops.export_scene.gltf(filepath=os.path.join(out, name.replace(".fbx", "")), export_format="GLB")

# 1. Textured, two-material cube moved off origin with a child.
reset()
bpy.ops.mesh.primitive_cube_add(size=2, location=(1, 2, 3))
cube = bpy.context.object
cube.name = "Crate"
cube.rotation_euler = (0, 0, math.radians(30))
cube.data.materials.append(textured_material("CrateWood"))
cube.data.materials.append(plain_material("CratePaint", (0.1, 0.8, 0.2, 1)))
for i, p in enumerate(cube.data.polygons):
    p.material_index = 1 if i == 0 else 0
bpy.ops.mesh.primitive_uv_sphere_add(segments=8, ring_count=4, radius=0.5, location=(0, 0, 2))
sph = bpy.context.object
sph.name = "Knob"
sph.parent = cube
sph.matrix_parent_inverse = cube.matrix_world.inverted()
export("crate.fbx", path_mode="RELATIVE")
export("crate_embedded.fbx", path_mode="COPY", embed_textures=True)

# 2. Skinned column with three bones and a 10-frame bend animation.
reset()
bpy.ops.mesh.primitive_cylinder_add(vertices=8, radius=0.3, depth=3, location=(0, 0, 1.5))
col = bpy.context.object
col.name = "Column"
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.subdivide(number_cuts=5)
bpy.ops.object.mode_set(mode="OBJECT")
col.data.materials.append(plain_material("Skin", (0.8, 0.5, 0.4, 1)))
bpy.ops.object.armature_add(location=(0, 0, 0))
arm = bpy.context.object
arm.name = "Rig"
bpy.ops.object.mode_set(mode="EDIT")
eb = arm.data.edit_bones
b0 = eb[0]; b0.name = "Root"; b0.head = (0, 0, 0); b0.tail = (0, 0, 1)
b1 = eb.new("Mid"); b1.head = (0, 0, 1); b1.tail = (0, 0, 2); b1.parent = b0; b1.use_connect = True
b2 = eb.new("Tip"); b2.head = (0, 0, 2); b2.tail = (0, 0, 3); b2.parent = b1; b2.use_connect = True
bpy.ops.object.mode_set(mode="OBJECT")
col.select_set(True); arm.select_set(True); bpy.context.view_layer.objects.active = arm
bpy.ops.object.parent_set(type="ARMATURE_AUTO")
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 11
bpy.ops.object.mode_set(mode="POSE")
for f, ang in ((1, 0), (6, 40), (11, 0)):
    for bn in ("Mid", "Tip"):
        pb = arm.pose.bones[bn]
        pb.rotation_mode = "XYZ"
        pb.rotation_euler = (math.radians(ang), 0, 0)
        pb.keyframe_insert("rotation_euler", frame=f)
bpy.ops.object.mode_set(mode="OBJECT")
arm.animation_data.action.name = "Bend"
export("bend.fbx", add_leaf_bones=False, bake_anim_use_all_actions=False, bake_anim_use_nla_strips=False)
# 3. The crate again under other axis systems and unit settings (from the glTF just written).
reset()
bpy.ops.import_scene.gltf(filepath=os.path.join(out, "crate.glb"))
bpy.ops.export_scene.fbx(filepath=os.path.join(out, "crate_zup_units.fbx"), axis_forward="Y", axis_up="Z",
                         apply_scale_options="FBX_SCALE_UNITS", path_mode="STRIP")
print("ok")

# 4. Blend shapes: a grid with two shape keys, one animated and one held at 0.25.
reset()
bpy.ops.mesh.primitive_grid_add(x_subdivisions=4, y_subdivisions=4, size=2, location=(0, 0, 0))
grid = bpy.context.object
grid.name = "Face"
grid.data.materials.append(plain_material("FaceSkin", (0.9, 0.7, 0.6, 1)))
grid.shape_key_add(name="Basis")
raise_key = grid.shape_key_add(name="Raise", from_mix=False)
for v in raise_key.data:
    if v.co.x > 0.1:
        v.co.z += 0.5
twist = grid.shape_key_add(name="Twist", from_mix=False)
for v in twist.data:
    v.co.x += 0.3 * v.co.y
twist.value = 0.25
scene = bpy.context.scene
scene.frame_start = 1; scene.frame_end = 11
for f, val in ((1, 0.0), (6, 1.0), (11, 0.0)):
    raise_key.value = val
    raise_key.keyframe_insert("value", frame=f)
export("morph.fbx", bake_anim_use_all_actions=False, bake_anim_use_nla_strips=False, bake_anim_simplify_factor=0.0)
print("ok")
