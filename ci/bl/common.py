# SPDX-License-Identifier: MIT
"""Shared scene helpers for the scripts in ci/bl/, which run INSIDE Blender.

Run them through ci/blender.py, which finds the binary, decodes the KTX2
textures Blender cannot read (ci/unbake_ktx.py) and passes the args after
`--`. Everything here builds objects through `bpy.data` rather than
operators, so it works in background mode and in the MCP add-on's timer
context, where `bpy.context.object` does not exist.

Cycles on the GPU is the default: a 512px/64spp frame is ~2 s on an RTX
3060 Ti where EEVEE spends 20 s compiling shaders. Set GATES_BL_CPU=1 to
force CPU on a box with no GPU.
"""
import math
import os
import sys

import bpy
from mathutils import Vector

__all__ = (
    "args_after_dashes",
    "bounds",
    "fit_camera",
    "ground",
    "import_glb",
    "look_at",
    "mesh_object",
    "new_scene",
    "plain_material",
    "render",
    "setup_render",
    "sky",
    "sun",
)

# The game's character capsule is 1.8 m; everything is framed against it.
PLAYER_H = 1.8


def args_after_dashes():
    argv = sys.argv
    return argv[argv.index("--") + 1:] if "--" in argv else []


def new_scene():
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    return bpy.context.scene


def import_glb(path, name=None):
    """Import a GLB, parent its roots to a fresh empty, return the empty
    and the mesh objects. The empty is what you move."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path, import_shading="NORMALS")
    new = [o for o in bpy.data.objects if o not in before]
    root = bpy.data.objects.new(name or os.path.basename(path)[:-4], None)
    bpy.context.scene.collection.objects.link(root)
    for o in new:
        if o.parent is None:
            o.parent = root
    bpy.context.view_layer.update()
    meshes = [o for o in new if o.type == "MESH"]
    # Drop the importer's emission: Meshy ships emissive maps on everything
    # (ci/import_meshy.py §2), and a glowing workbench spoils a daylight sheet.
    for o in meshes:
        for slot in o.material_slots:
            m = slot.material
            if m and m.use_nodes:
                for n in m.node_tree.nodes:
                    if n.type == "BSDF_PRINCIPLED":
                        n.inputs["Emission Strength"].default_value = 0.0
    return root, meshes


def bounds(objs):
    pts = [o.matrix_world @ Vector(c) for o in objs if o.type == "MESH"
           for c in o.bound_box]
    if not pts:
        return Vector((0, 0, 0)), Vector((0, 0, 0))
    lo = Vector([min(p[i] for p in pts) for i in range(3)])
    hi = Vector([max(p[i] for p in pts) for i in range(3)])
    return lo, hi


def ground(scene, size=400.0, color=(0.16, 0.15, 0.13), rough=0.95):
    me = bpy.data.meshes.new("ground")
    s = size / 2
    me.from_pydata([(-s, -s, 0), (s, -s, 0), (s, s, 0), (-s, s, 0)], [], [(0, 1, 2, 3)])
    ob = bpy.data.objects.new("ground", me)
    scene.collection.objects.link(ob)
    m = bpy.data.materials.new("ground")
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1)
    bsdf.inputs["Roughness"].default_value = rough
    me.materials.append(m)
    return ob


def sky(scene, elevation=55.0, rotation=210.0, strength=0.3, turbidity=3.0):
    w = bpy.data.worlds.new("sky")
    scene.world = w
    w.use_nodes = True
    nt = w.node_tree
    tex = nt.nodes.new("ShaderNodeTexSky")
    # Blender 5 renamed Nishita to the scattering models; 4.x still has it.
    for kind in ("MULTIPLE_SCATTERING", "NISHITA"):
        try:
            tex.sky_type = kind
            break
        except TypeError:
            continue
    tex.sun_elevation = math.radians(elevation)
    tex.sun_rotation = math.radians(rotation)
    tex.sun_intensity = 1.0
    if hasattr(tex, "dust_density"):
        tex.dust_density = turbidity
    bg = nt.nodes["Background"]
    nt.links.new(tex.outputs[0], bg.inputs[0])
    bg.inputs[1].default_value = strength
    return tex


def sun(scene, elevation=55.0, rotation=210.0, strength=4.0, angle=1.0):
    la = bpy.data.lights.new("sun", "SUN")
    la.energy = strength
    la.angle = math.radians(angle)
    ob = bpy.data.objects.new("sun", la)
    scene.collection.objects.link(ob)
    ob.rotation_euler = (math.radians(90 - elevation), 0, math.radians(rotation))
    return ob


def fit_camera(scene, lo, hi, azimuth=35.0, pitch=22.0, lens=50.0, fill=0.82,
               aspect=None):
    """Aim a camera at the box's centre from (azimuth, pitch) and dolly it
    until the box's corners fill `fill` of the frame."""
    cam = scene.camera
    if cam is None:
        cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
        scene.collection.objects.link(cam)
        scene.camera = cam
    cam_data = cam.data
    cam_data.lens = lens
    cam_data.clip_end = 5000
    ctr = (lo + hi) / 2
    az, pt = math.radians(azimuth), math.radians(pitch)
    d = Vector((math.sin(az) * math.cos(pt), -math.cos(az) * math.cos(pt), math.sin(pt)))
    cam.rotation_euler = (-d).to_track_quat("-Z", "Y").to_euler()
    corners = [Vector((x, y, z)) for x in (lo.x, hi.x) for y in (lo.y, hi.y) for z in (lo.z, hi.z)]
    rx = scene.render.resolution_x
    ry = scene.render.resolution_y
    aspect = aspect or rx / ry
    sw = cam_data.sensor_width
    fx = lens / (sw / 2)
    fy = fx * aspect if aspect >= 1 else fx
    if aspect < 1:
        fx = fy / aspect
    # Distance along -d so every corner's projected extent is within fill.
    rot = cam.rotation_euler.to_matrix().transposed()
    need = 0.0
    for c in corners:
        v = rot @ (c - ctr)
        # v.z is depth along the camera's -Z; |x| * fx / depth <= fill.
        need = max(need, abs(v.x) * fx / fill + v.z, abs(v.y) * fy / fill + v.z)
    cam.location = ctr + d * need
    return cam


def look_at(scene, eye, target, lens=35.0):
    cam = scene.camera
    if cam is None:
        cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
        scene.collection.objects.link(cam)
        scene.camera = cam
    cam.data.lens = lens
    cam.data.clip_end = 5000
    cam.location = Vector(eye)
    cam.rotation_euler = (Vector(target) - Vector(eye)).to_track_quat("-Z", "Y").to_euler()
    return cam


def mesh_object(name, bm, material=None):
    """A mesh object from a bmesh, linked to the scene."""
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    ob = bpy.data.objects.new(name, me)
    bpy.context.scene.collection.objects.link(ob)
    if material is not None:
        me.materials.append(material)
    return ob


def plain_material(name, color, rough=0.9, noise=0.0, noise_scale=2.0, bump=0.0):
    """A Principled material; `noise` mixes the colour toward a darker copy
    by a noise texture, `bump` adds the same noise as surface relief."""
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1)
    bsdf.inputs["Roughness"].default_value = rough
    if noise > 0 or bump > 0:
        tex = nt.nodes.new("ShaderNodeTexNoise")
        tex.inputs["Scale"].default_value = noise_scale
        tex.inputs["Detail"].default_value = 6.0
        if noise > 0:
            mix = nt.nodes.new("ShaderNodeMix")
            mix.data_type = "RGBA"
            mix.inputs["Factor"].default_value = noise
            mix.inputs[6].default_value = (*color, 1)
            mix.inputs[7].default_value = (*[c * 0.45 for c in color], 1)
            nt.links.new(tex.outputs["Fac"], mix.inputs["Factor"])
            nt.links.new(mix.outputs[2], bsdf.inputs["Base Color"])
        if bump > 0:
            b = nt.nodes.new("ShaderNodeBump")
            b.inputs["Strength"].default_value = bump
            nt.links.new(tex.outputs["Fac"], b.inputs["Height"])
            nt.links.new(b.outputs["Normal"], bsdf.inputs["Normal"])
    return m


def setup_render(scene, w, h, samples=96, exposure=-0.5, transparent=False):
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU" if os.environ.get("GATES_BL_CPU") else "GPU"
    scene.cycles.samples = samples
    scene.cycles.use_denoising = True
    scene.cycles.use_adaptive_sampling = True
    scene.render.resolution_x = w
    scene.render.resolution_y = h
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = transparent
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Base Contrast"
    scene.view_settings.exposure = exposure
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA" if transparent else "RGB"
    _enable_gpu()


def _enable_gpu():
    prefs = bpy.context.preferences.addons.get("cycles")
    if not prefs:
        return
    cp = prefs.preferences
    for kind in ("OPTIX", "CUDA", "HIP", "METAL", "ONEAPI"):
        try:
            cp.compute_device_type = kind
            cp.refresh_devices()
        except TypeError:
            continue
        if any(d.type == kind for d in cp.devices):
            break
    for d in cp.devices:
        d.use = d.type != "CPU"


def render(scene, path):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    return path
