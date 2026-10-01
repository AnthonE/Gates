"""The objects `ci/prop_kit.py` builds. One function per model; each places
parts on a `Kit` in Blender space (metres, Z up, a deployable's front
toward −Y). `size` is the `DEPLOY` row (w, h, d) a deployable is fitted
inside; a held item's height is whatever it measures, and its `HELD_MODELS`
row restates it.
"""
import math

RECIPES = {}


def recipe(name, out, kind="deploy", size=None, tex=1024):
    def deco(fn):
        RECIPES[name] = dict(fn=fn, out=out, kind=kind, size=size, tex=tex)
        return fn
    return deco


# ── deployables ─────────────────────────────────────────────────────────────

@recipe("furnace", "models/deploy/furnace.glb", size=(1.3, 0.95, 0.85))
def furnace(k):
    # Plinth, a stacked-stone body and a narrower hood, all stone.
    k.box((1.06, 0.80, 0.08), (0, 0, 0.04), "stone", bevel=0.02, segs=2)
    k.box((0.96, 0.70, 0.50), (0, 0, 0.33), "stone", bevel=0.035, segs=2)
    k.box((0.80, 0.58, 0.14), (0, 0.02, 0.65), "stone", bevel=0.035, segs=2)
    # Iron straps round the body.
    for z in (0.17, 0.50):
        k.box((0.98, 0.72, 0.035), (0, 0, z), "iron", bevel=0.004)
    # Chimney, back left, with a lip.
    k.cyl(0.11, 0.26, (-0.2, 0.12, 0.82), "iron", r2=0.09, n=14)
    k.torus(0.095, 0.018, (-0.2, 0.12, 0.95 - 0.018), "iron", n=14, m=6)
    # The mouth: an iron frame and a closed door with vent slots — an open
    # door would stand proud of the row and shrink the whole model to fit.
    for x in (-0.22, 0.22):
        k.box((0.04, 0.04, 0.32), (x, -0.36, 0.32), "iron", bevel=0.004)
    for z in (0.18, 0.46):
        k.box((0.48, 0.04, 0.04), (0, -0.36, z), "iron", bevel=0.004)
    k.box((0.40, 0.025, 0.24), (0, -0.37, 0.32), "rusty", bevel=0.004)
    for z in (0.28, 0.32, 0.36):
        k.box((0.26, 0.006, 0.016), (0.02, -0.384, z), "soot", bevel=0.0)
    for x in (-0.14, 0.14):
        k.cyl(0.02, 0.03, (-0.18, -0.385, 0.32 + x * 0.6), "dark", axis="Y", n=8)
    k.cyl(0.012, 0.05, (0.15, -0.40, 0.32), "dark", axis="Y", n=8)
    # Ash tray under the mouth.
    k.box((0.36, 0.08, 0.05), (0, -0.41, 0.105), "iron", bevel=0.004)
    # A charcoal bin against the right side.
    k.box((0.24, 0.40, 0.26), (0.62, -0.05, 0.13), "darkwood", bevel=0.01)
    k.box((0.20, 0.36, 0.04), (0.62, -0.05, 0.25), "char", bevel=0.0)
    for i in range(5):
        k.ball(0.035, (0.56 + (i % 3) * 0.05, -0.15 + i * 0.05, 0.28), "char", n=6)


@recipe("recycler", "models/deploy/recycler.glb", size=(1.3, 1.15, 0.9))
def recycler(k):
    # Squat painted machine on a steel skid; the hopper is the silhouette.
    for x in (-0.5, 0.5):
        for y in (-0.3, 0.3):
            k.box((0.1, 0.1, 0.06), (x, y, 0.03), "dark", bevel=0.006)
    k.box((1.16, 0.78, 0.08), (0, 0, 0.10), "dark", bevel=0.006)
    k.box((1.04, 0.70, 0.60), (0, 0, 0.44), "green", bevel=0.025, segs=2)
    k.box((1.06, 0.72, 0.05), (0, 0, 0.72), "dark", bevel=0.006)
    # Hopper: a square funnel, open, with a floor. n=4 lathe turned 45°.
    k.lathe([(0.30, 0.74), (0.52, 1.08), (0.49, 1.08), (0.27, 0.77), (0.0, 0.77)],
            surf="yellow", n=4, rot=(0, 0, 45))
    k.lathe([(0.535, 1.075), (0.535, 1.10), (0.49, 1.10), (0.49, 1.075)], surf="dark", n=4, rot=(0, 0, 45))
    # Warning band across the front: alternating yellow and black.
    for i in range(10):
        k.box((0.1, 0.012, 0.06), (-0.45 + i * 0.1, -0.354, 0.66), "yellow" if i % 2 else "rubber", bevel=0.0)
    # Flywheel on the right side.
    k.torus(0.22, 0.025, (0.56, 0.0, 0.46), "iron", axis="X", n=24, m=6)
    for a in range(0, 180, 45):
        k.box((0.02, 0.42, 0.03), (0.56, 0, 0.46), "iron", rot=(a, 0, 0), bevel=0.003)
    k.cyl(0.05, 0.08, (0.56, 0, 0.46), "dark", axis="X", n=12)
    k.cyl(0.015, 0.14, (0.62, 0.0, 0.62), "dark", axis="X", n=8)
    # Output chute at the front, sloping down, with cheeks.
    k.box((0.42, 0.24, 0.025), (0, -0.44, 0.24), "sheet", rot=(-25, 0, 0), bevel=0.003)
    for x in (-0.21, 0.21):
        k.box((0.02, 0.24, 0.08), (x, -0.44, 0.27), "sheet", rot=(-25, 0, 0), bevel=0.003)
    k.box((0.46, 0.02, 0.16), (0, -0.355, 0.32), "soot", bevel=0.0)
    # Big red button and a lever on the left.
    k.cyl(0.05, 0.03, (-0.35, -0.36, 0.48), "dark", axis="Y", n=14)
    k.cyl(0.035, 0.03, (-0.35, -0.38, 0.48), "red", axis="Y", n=14)
    k.box((0.05, 0.06, 0.05), (-0.55, 0.0, 0.40), "dark", bevel=0.004)
    k.cyl(0.012, 0.28, (-0.6, 0.0, 0.52), "steel", rot=(0, -20, 0), n=8)
    k.ball(0.03, (-0.65, 0.0, 0.65), "rubber", n=8)
    # Exhaust stack at the back.
    k.cyl(0.05, 0.36, (0.32, 0.26, 0.86), "rusty", n=12)
    k.cyl(0.065, 0.04, (0.32, 0.26, 1.04), "dark", n=12)
    # Rivets on the front corners.
    for x in (-0.48, 0.48):
        for z in (0.2, 0.62):
            k.cyl(0.012, 0.01, (x, -0.352, z), "steel", axis="Y", n=6, bevel=0.0)


@recipe("research_table", "models/deploy/research_table.glb", size=(1.5, 0.8, 0.8))
def research_table(k):
    top = 0.66
    k.box((1.46, 0.72, 0.05), (0, 0, top - 0.025), "darkwood", bevel=0.008)
    for x in (-0.66, 0.66):
        for y in (-0.32, 0.32):
            k.box((0.06, 0.06, top - 0.05), (x, y, (top - 0.05) / 2), "iron", bevel=0.004)
    for y in (-0.32, 0.32):
        k.box((1.30, 0.04, 0.05), (0, y, 0.12), "iron", bevel=0.004)
    k.box((1.32, 0.64, 0.025), (0, 0, 0.16), "planks", bevel=0.004)
    k.box((0.3, 0.22, 0.2), (-0.4, 0.05, 0.27), "darkwood", bevel=0.01)
    # On top: papers, a blueprint, an open book, calipers, a vice, a lamp.
    for i, (x, y, a) in enumerate([(-0.35, -0.12, 8), (-0.28, -0.05, -14), (-0.42, 0.02, 21)]):
        k.box((0.21, 0.297, 0.003), (x, y, top + 0.002 + i * 0.003), "paper", rot=(0, 0, a), bevel=0.0)
    k.box((0.42, 0.30, 0.003), (0.08, -0.08, top + 0.002), "blueprint", rot=(0, 0, -4), bevel=0.0)
    k.box((0.34, 0.24, 0.012), (0.10, 0.20, top + 0.006), "leather", rot=(0, 0, 10), bevel=0.002)
    for s in (-1, 1):
        k.box((0.155, 0.22, 0.012), (0.10 + s * 0.08, 0.20, top + 0.018), "paper", rot=(0, s * -6, 10), bevel=0.001)
    k.box((0.012, 0.16, 0.004), (0.32, -0.15, top + 0.003), "steel", rot=(0, 0, 30), bevel=0.0)
    k.box((0.04, 0.012, 0.004), (0.33, -0.08, top + 0.003), "steel", rot=(0, 0, 30), bevel=0.0)
    # Vice clamped over the front-right edge.
    vx = 0.55
    k.box((0.16, 0.12, 0.05), (vx, -0.30, top + 0.025), "blue", bevel=0.006)
    k.box((0.16, 0.04, 0.08), (vx, -0.36, top + 0.09), "blue", bevel=0.006)
    k.box((0.16, 0.04, 0.08), (vx, -0.27, top + 0.09), "blue", bevel=0.006)
    k.cyl(0.012, 0.10, (vx, -0.36, top + 0.07), "steel", axis="Y", n=8)
    k.cyl(0.008, 0.14, (vx, -0.405, top + 0.07), "steel", axis="X", n=6)
    # Lamp: a weighted base, an arm, a shade.
    k.cyl(0.07, 0.02, (-0.58, 0.24, top + 0.01), "dark", n=14)
    k.cyl(0.008, 0.12, (-0.58, 0.24, top + 0.08), "dark", n=6)
    k.lathe([(0.0, 0.0), (0.02, 0.0), (0.06, -0.06), (0.055, -0.06), (0.0, -0.005)], at=(-0.55, 0.20, top + 0.13),
            surf="green", n=14, rot=(25, 0, -40))


@recipe("workbench2", "models/deploy/workbench2.glb", size=(1.6, 1.0, 0.8))
def workbench2(k):
    # The wood bench grown a tool board: wood frame, metal fittings.
    top = 0.80
    k.box((1.56, 0.66, 0.06), (0, -0.05, top - 0.03), "planks", bevel=0.008)
    for x in (-0.72, 0.72):
        for y in (-0.32, 0.22):
            k.box((0.08, 0.08, top - 0.06), (x, y, (top - 0.06) / 2), "wood", bevel=0.006)
            k.box((0.09, 0.09, 0.06), (x, y, 0.03), "iron", bevel=0.004)
    for y in (-0.32, 0.22):
        k.box((1.40, 0.05, 0.08), (0, y, 0.2), "wood", bevel=0.005)
    k.box((1.40, 0.52, 0.03), (0, -0.05, 0.25), "planks", bevel=0.004)
    # Back board, steel corner plates, a shelf, and tools on pegs.
    k.box((1.56, 0.04, 0.20), (0, 0.30, 0.90), "planks", bevel=0.005)
    for x in (-0.74, 0.74):
        k.box((0.06, 0.06, 0.24), (x, 0.30, 0.88), "wood", bevel=0.005)
        k.box((0.07, 0.07, 0.07), (x, -0.36, top - 0.035), "iron", bevel=0.004)
    k.box((1.5, 0.12, 0.025), (0, 0.24, 0.99), "wood", bevel=0.004)
    # A saw, a wrench, a coil of rope on the board.
    k.slab([(-0.18, 0), (0.12, 0), (0.12, 0.08), (-0.18, 0.05)], 0.004, (-0.4, 0.275, 0.86), "steel", bevel=0.0)
    k.box((0.07, 0.025, 0.08), (-0.6, 0.275, 0.89), "wood", bevel=0.006)
    k.box((0.02, 0.01, 0.16), (0.2, 0.275, 0.88), "steel", rot=(0, 20, 0), bevel=0.002)
    k.torus(0.08, 0.018, (0.5, 0.27, 0.88), "rope", axis="Y", n=16, m=6)
    # On the top: a vice and a few planks.
    k.box((0.14, 0.12, 0.06), (0.55, -0.28, top + 0.03), "iron", bevel=0.005)
    k.box((0.14, 0.035, 0.07), (0.55, -0.34, top + 0.095), "iron", bevel=0.005)
    k.box((0.14, 0.035, 0.07), (0.55, -0.24, top + 0.095), "iron", bevel=0.005)
    k.cyl(0.01, 0.08, (0.55, -0.36, top + 0.08), "steel", axis="Y", n=8)
    for i in range(3):
        k.box((0.7, 0.12, 0.025), (-0.25 + i * 0.03, -0.12 + i * 0.13, top + 0.0125 + 0.002 * i), "wood",
              rot=(0, 0, 3 - i * 4), bevel=0.003)


@recipe("workbench3", "models/deploy/workbench3.glb", size=(1.8, 1.1, 0.8))
def workbench3(k):
    # The machine bench: a steel table with a drill press and a grinder.
    top = 0.84
    k.box((1.76, 0.76, 0.05), (0, 0, top - 0.025), "sheet", bevel=0.006)
    for x in (-0.82, 0.82):
        for y in (-0.33, 0.33):
            k.box((0.07, 0.07, top - 0.05), (x, y, (top - 0.05) / 2), "green", bevel=0.005)
    k.box((1.70, 0.70, 0.04), (0, 0, 0.18), "dark", bevel=0.004)
    # A drawer cabinet under the left end.
    k.box((0.5, 0.62, 0.52), (-0.5, 0.02, 0.48), "green", bevel=0.012)
    for z in (0.36, 0.6):
        k.box((0.44, 0.01, 0.2), (-0.5, -0.295, z), "green", bevel=0.004)
        k.box((0.14, 0.02, 0.02), (-0.5, -0.31, z + 0.06), "steel", bevel=0.003)
    # Drill press, back right: base, column, head, motor, chuck, handles.
    dx, dy = 0.45, 0.12
    k.box((0.26, 0.30, 0.04), (dx, dy, top + 0.02), "dark", bevel=0.006)
    k.cyl(0.03, 0.26, (dx, dy + 0.09, top + 0.17), "steel", n=12)
    k.box((0.16, 0.30, 0.10), (dx, dy, top + 0.20), "green", bevel=0.015)
    k.cyl(0.055, 0.12, (dx, dy + 0.18, top + 0.22), "dark", axis="Y", n=14)
    k.cyl(0.02, 0.06, (dx, dy - 0.08, top + 0.12), "steel", n=10)
    k.cyl(0.006, 0.05, (dx, dy - 0.08, top + 0.07), "steel", n=6)
    for a in (0, 120, 240):
        k.cyl(0.006, 0.14, (dx + 0.12, dy - 0.02, top + 0.2), "steel", axis="Y", rot=(0, 0, a - 30), n=6)
    # Bench grinder, front left of the top.
    gx = -0.2
    k.box((0.10, 0.12, 0.10), (gx, -0.18, top + 0.07), "green", bevel=0.012)
    for s in (-1, 1):
        k.cyl(0.07, 0.025, (gx + s * 0.10, -0.18, top + 0.12), "concrete", axis="X", n=16)
        k.lathe([(0.075, -0.02), (0.08, -0.02), (0.08, 0.02), (0.075, 0.02)], (gx + s * 0.10, -0.18, top + 0.12),
                "dark", axis="X", n=16)
    # Back rail with hanging wrenches and a steel back panel.
    k.box((1.76, 0.03, 0.22), (0, 0.37, 0.98), "sheet", bevel=0.004)
    for i in range(4):
        k.box((0.018, 0.008, 0.14 - i * 0.02), (-0.7 + i * 0.07, 0.35, 0.96), "steel", bevel=0.002)


# ── held items ──────────────────────────────────────────────────────────────
# Authored standing: the grip on the Z axis (x = y = 0), foot at z = 0, a
# head's long axis along +X (`ui::hold::HAFTED_LAY`'s assumption).

@recipe("metal_hatchet", "models/held/metal_hatchet.glb", kind="held")
def metal_hatchet(k):
    k.lathe([(0.0, 0.0), (0.019, 0.0), (0.021, 0.02), (0.016, 0.08), (0.015, 0.30), (0.0105, 0.46),
             (0.0, 0.47)], surf="wood", n=10)
    k.cyl(0.0195, 0.13, (0, 0, 0.095), "leather", n=10, bevel=0.0)
    # Forged head: an eye round the haft, a cheek flaring to a bearded edge.
    head = [(-0.05, 0.455), (0.03, 0.455), (0.09, 0.43), (0.135, 0.395), (0.16, 0.40),
            (0.17, 0.47), (0.165, 0.548), (0.12, 0.535), (0.03, 0.525), (-0.05, 0.525)]
    k.slab(head, 0.024, (0, 0, 0), "dark", bevel=0.004)
    k.slab([(0.15, 0.402), (0.162, 0.40), (0.172, 0.47), (0.167, 0.546), (0.155, 0.543), (0.158, 0.47)],
           0.012, (0, 0, 0), "steel", bevel=0.001)
    k.box((0.03, 0.032, 0.075), (-0.035, 0, 0.49), "dark", bevel=0.006)


@recipe("metal_pickaxe", "models/held/metal_pickaxe.glb", kind="held")
def metal_pickaxe(k):
    k.lathe([(0.0, 0.0), (0.02, 0.0), (0.022, 0.02), (0.017, 0.10), (0.016, 0.40), (0.019, 0.60),
             (0.0, 0.62)], surf="wood", n=10)
    k.cyl(0.021, 0.14, (0, 0, 0.10), "leather", n=10, bevel=0.0)
    k.box((0.07, 0.05, 0.08), (0, 0, 0.6), "dark", bevel=0.008)
    # Pick to +X curving down to a point, adze to −X flattening to an edge.
    k.tube([(0.03, 0, 0.61), (0.12, 0, 0.615), (0.20, 0, 0.59), (0.27, 0, 0.545)], 0.02, "dark", n=8, r_end=0.003)
    adze = [(-0.03, 0.585), (-0.03, 0.635), (-0.15, 0.62), (-0.25, 0.575), (-0.255, 0.555), (-0.15, 0.585)]
    k.slab(adze, 0.04, (0, 0, 0), "dark", bevel=0.004)
    k.slab([(-0.245, 0.556), (-0.262, 0.554), (-0.256, 0.578), (-0.24, 0.578)], 0.044, (0, 0, 0), "steel", bevel=0.0)


@recipe("metal_spear", "models/held/metal_spear.glb", kind="held")
def metal_spear(k):
    k.lathe([(0.0, 0.0), (0.017, 0.0), (0.018, 0.02), (0.016, 0.9), (0.015, 1.76), (0.0, 1.78)], surf="wood", n=10)
    k.lathe([(0.0, 1.70), (0.02, 1.70), (0.022, 1.74), (0.017, 1.80), (0.0, 1.80)], surf="iron", n=10)
    # Leaf blade: a diamond-section lathe flattened across Y.
    k.lathe([(0.0, 1.79), (0.02, 1.80), (0.042, 1.86), (0.036, 1.92), (0.0, 2.0)], surf="steel", n=4,
            rot=(0, 0, 45), scale=(1.0, 0.28, 1.0))
    k.tube([(0, 0, 1.66 + i * 0.012) for i in range(4)], 0.0195, "rope", n=10)
    for i in range(5):
        k.torus(0.0175, 0.004, (0, 0, 1.62 + i * 0.012), "rope", n=12, m=5)


@recipe("satchel_charge", "models/held/satchel_charge.glb", kind="held", tex=512)
def satchel_charge(k):
    k.box((0.22, 0.12, 0.17), (0, 0, 0.085), "burlap", bevel=0.03, segs=3)
    k.box((0.225, 0.125, 0.06), (0, 0.004, 0.15), "burlap", rot=(-6, 0, 0), bevel=0.02, segs=2)
    for x in (-0.06, 0.06):
        k.box((0.04, 0.128, 0.175), (x, 0, 0.085), "tape", bevel=0.01, segs=2)
    # The fuse: a cord out of the top with a brass igniter.
    k.tube([(0.05, 0, 0.17), (0.06, 0, 0.21), (0.03, 0, 0.25), (0.0, 0, 0.27)], 0.005, "rubber", n=6)
    k.cyl(0.009, 0.03, (0.0, 0, 0.28), "brass", n=8)
    # Carry strap over the top.
    k.torus(0.075, 0.008, (0, 0, 0.17), "olive", axis="Y", n=16, m=5, arc=180)


@recipe("bandage", "models/held/bandage.glb", kind="held", tex=512)
def bandage(k):
    k.cyl(0.035, 0.07, (0, 0, 0.035), "gauze", n=18, bevel=0.004)
    k.cyl(0.012, 0.072, (0, 0, 0.035), "white", n=10, bevel=0.0)
    k.box((0.07, 0.002, 0.066), (0.034, -0.03, 0.033), "gauze", rot=(0, 0, -20), bevel=0.0)


@recipe("medkit", "models/held/medkit.glb", kind="held", tex=512)
def medkit(k):
    k.box((0.24, 0.09, 0.17), (0, 0, 0.085), "olive", bevel=0.025, segs=3)
    k.box((0.245, 0.095, 0.05), (0, 0.003, 0.15), "olive", bevel=0.015, segs=2)
    # ISO 7010 E003 (first aid): a white cross on green — a safety sign, not
    # any organisation's emblem.
    k.box((0.08, 0.004, 0.08), (0, -0.047, 0.08), "signgreen", bevel=0.002)
    k.box((0.05, 0.004, 0.016), (0, -0.0495, 0.08), "signwhite", bevel=0.0)
    k.box((0.016, 0.004, 0.05), (0, -0.0505, 0.08), "signwhite", bevel=0.0)
    k.box((0.10, 0.03, 0.012), (0, 0, 0.176), "leather", bevel=0.004)
    k.cyl(0.01, 0.02, (0, -0.048, 0.155), "brass", axis="Y", n=8)


@recipe("berries", "models/held/berries.glb", kind="held", tex=512)
def berries(k):
    import random
    rnd = random.Random(7)
    pts = []
    for i in range(14):
        a = rnd.uniform(0, 2 * math.pi)
        r = rnd.uniform(0.0, 0.03)
        pts.append((r * math.cos(a), r * math.sin(a), 0.012 + rnd.uniform(0, 0.05)))
    for p in pts:
        k.ball(rnd.uniform(0.009, 0.013), p, "berry", n=8)
    k.tube([(0, 0, 0.02), (0.005, 0, 0.06), (0.0, 0.004, 0.09)], 0.002, "leaf", n=5)
    for a in (0, 130, 250):
        k.slab([(0, 0), (0.012, 0.015), (0, 0.04), (-0.012, 0.015)], 0.002, (0, 0, 0.075), "leaf",
               rot=(-50, 0, a), bevel=0.0)


@recipe("mushrooms", "models/held/mushrooms.glb", kind="held", tex=512)
def mushrooms(k):
    for (x, y, h, r, tilt) in ((0.0, 0.0, 0.09, 0.035, 0), (0.03, 0.01, 0.065, 0.026, 18), (-0.025, -0.012, 0.055, 0.022, -22)):
        stem = k.lathe([(0.0, 0.0), (0.011, 0.0), (0.008, h * 0.5), (0.007, h), (0.0, h)], surf="stem", n=10)
        stem.location = (x, y, 0)
        stem.rotation_euler = (0, math.radians(tilt), 0)
        cap = k.lathe([(0.0, h - 0.008), (r * 0.7, h - 0.01), (r, h - 0.002), (r * 0.8, h + 0.012), (0.0, h + 0.02)],
                      surf="cap", n=14)
        cap.location = (x, y, 0)
        cap.rotation_euler = (0, math.radians(tilt), 0)


@recipe("corn", "models/held/corn.glb", kind="held", tex=512)
def corn(k):
    k.lathe([(0.0, 0.0), (0.012, 0.0), (0.022, 0.02), (0.025, 0.08), (0.022, 0.15), (0.012, 0.19), (0.0, 0.2)],
            surf="corn", n=14)
    for i, a in enumerate((0, 120, 240)):
        leaf = [(-0.02, 0.0), (0.02, 0.0), (0.016, 0.09), (0.0, 0.14 - i * 0.02), (-0.016, 0.09)]
        k.slab(leaf, 0.002, (0, 0, 0), "husk", rot=(0, 0, a), bevel=0.0).location = (0.02 * math.cos(math.radians(a)), 0.02 * math.sin(math.radians(a)), 0)
    k.cyl(0.01, 0.03, (0, 0, -0.0), "husk", n=8)


def _drumstick(k, surf):
    k.lathe([(0.0, 0.0), (0.012, 0.0), (0.014, 0.008), (0.009, 0.02), (0.008, 0.05), (0.0, 0.06)], surf="bone", n=10)
    k.lathe([(0.0, 0.045), (0.016, 0.05), (0.035, 0.09), (0.045, 0.14), (0.04, 0.18), (0.022, 0.2), (0.0, 0.205)],
            surf=surf, n=14, scale=(1.0, 0.82, 1.0))


@recipe("raw_meat", "models/held/raw_meat.glb", kind="held", tex=512)
def raw_meat(k):
    _drumstick(k, "raw")


@recipe("cooked_meat", "models/held/cooked_meat.glb", kind="held", tex=512)
def cooked_meat(k):
    _drumstick(k, "cooked")


@recipe("burnt_meat", "models/held/burnt_meat.glb", kind="held", tex=512)
def burnt_meat(k):
    _drumstick(k, "burnt")


def _keycard(k, surf):
    k.box((0.054, 0.0024, 0.0856), (0, 0, 0.0428), surf, bevel=0.003, segs=2)
    k.box((0.054, 0.0026, 0.012), (0, 0, 0.068), "rubber", bevel=0.0)
    k.box((0.012, 0.0028, 0.010), (-0.012, 0, 0.035), "brass", bevel=0.0005)
    k.box((0.03, 0.0027, 0.004), (0.005, 0, 0.018), "white", bevel=0.0)


@recipe("green_keycard", "models/held/green_keycard.glb", kind="held", tex=512)
def green_keycard(k):
    _keycard(k, "kgreen")


@recipe("blue_keycard", "models/held/blue_keycard.glb", kind="held", tex=512)
def blue_keycard(k):
    _keycard(k, "kblue")


@recipe("red_keycard", "models/held/red_keycard.glb", kind="held", tex=512)
def red_keycard(k):
    _keycard(k, "kred")


# ── misc ────────────────────────────────────────────────────────────────────

@recipe("death_bag", "models/prop/death_bag.glb", kind="centre", size=(0.6, 0.35, 0.45))
def death_bag(k):
    # A rucksack dropped on its back, in the tan the cuboid wore so it still
    # reads against grass, with a rolled blanket strapped across the top.
    k.box((0.50, 0.36, 0.20), (0, 0, 0.10), "tan", bevel=0.06, segs=3)
    k.box((0.46, 0.30, 0.06), (0, -0.02, 0.21), "tan", rot=(4, 0, 0), bevel=0.025, segs=2)
    k.box((0.18, 0.08, 0.14), (0.0, -0.20, 0.09), "tan", bevel=0.03, segs=2)
    k.cyl(0.07, 0.56, (0, 0.13, 0.26), "olive", axis="X", n=16, bevel=0.015, segs=2)
    for x in (-0.16, 0.16):
        k.torus(0.075, 0.009, (x, 0.13, 0.26), "leather", axis="X", n=14, m=5)
        k.box((0.04, 0.36, 0.006), (x, -0.02, 0.243), "leather", rot=(4, 0, 0), bevel=0.0)
        k.box((0.03, 0.012, 0.02), (x, -0.20, 0.22), "brass", bevel=0.002)
    for x in (-0.12, 0.12):
        k.tube([(x, 0.17, 0.02), (x * 1.3, 0.24, 0.05), (x * 1.4, 0.26, 0.15), (x, 0.19, 0.19)], 0.012, "leather", n=6)


def _door(k, leaf, strap):
    # Thin along X, wide along Y, 2.1 tall: five boards, two ledges and a
    # brace behind, strap hinges and a ring pull in front. The locked
    # variant is this geometry under other surfaces, so the two share one
    # unwrap and `build_kit` can swap materials on one mesh.
    for i in range(5):
        k.box((0.05, 0.172, 2.08), (0, -0.36 + i * 0.18, 1.04), leaf, bevel=0.008)
    for z in (0.35, 1.75):
        k.box((0.035, 0.84, 0.14), (0.042, 0, z), leaf, bevel=0.006)
    k.box((0.035, 0.12, 1.55), (0.042, 0, 1.05), leaf, rot=(-26.5, 0, 0), bevel=0.006)
    for z in (0.35, 1.75):
        k.box((0.012, 0.62, 0.05), (-0.031, 0.13, z), strap, bevel=0.003)
        k.cyl(0.022, 0.10, (-0.012, 0.43, z), strap, n=10)
        for y in (-0.1, 0.1, 0.3):
            k.cyl(0.009, 0.008, (-0.038, y, z), strap, axis="X", n=6, bevel=0.0)
    k.cyl(0.03, 0.012, (-0.032, -0.33, 1.0), strap, axis="X", n=12)
    k.torus(0.04, 0.007, (-0.045, -0.33, 0.97), strap, axis="X", n=14, m=5)


@recipe("door", "models/deploy/door.glb", size=(0.12, 2.1, 0.9))
def door(k):
    _door(k, "planks", "iron")


@recipe("door_locked", "models/deploy/door_locked.glb", size=(0.12, 2.1, 0.9))
def door_locked(k):
    _door(k, "sheet", "dark")


# ── animals ─────────────────────────────────────────────────────────────────
# Authored in the glTF frame of `render/mobs.rs` (+Z forward, feet at 0, a
# leg's hip at its origin) and exported as-is, so the anchor tables there
# still say where each leg hangs. `G` turns that frame into Blender's.

def G(x, y, z):
    return (x, -z, y)


def _pig_body(k):
    hide = "pighide"
    k.lathe([(0.0, -0.57), (0.13, -0.55), (0.21, -0.47), (0.25, -0.25), (0.26, 0.05), (0.24, 0.32),
             (0.18, 0.46), (0.0, 0.50)], G(0, 0.52, 0), hide, axis="Y", n=14, scale=(1.0, 1.0, 0.86))
    k.ball(0.22, G(0, 0.60, 0.16), hide, scale=(1.05, 1.25, 0.78), n=12)
    k.lathe([(0.0, 0.36), (0.16, 0.40), (0.165, 0.52), (0.13, 0.66), (0.09, 0.80), (0.08, 0.855),
             (0.0, 0.86)], G(0, 0.0, 0.0), hide, axis="Y", n=12, scale=(1.0, 1.0, 0.92)).location = G(0, 0.50, 0)
    k.cyl(0.072, 0.025, G(0, 0.47, 0.865), "skin", axis="Y", n=12, bevel=0.006)
    for s in (-1, 1):
        k.ball(0.009, G(s * 0.03, 0.47, 0.88), "eye", n=6)
        k.ball(0.014, G(s * 0.095, 0.585, 0.70), "eye", n=6)
        k.slab([(-0.045, 0.0), (0.045, 0.0), (0.0, 0.10)], 0.012, G(s * 0.11, 0.66, 0.56), hide,
               plane="XZ", rot=(15, s * 25, 0), bevel=0.0)
        k.cyl(0.012, 0.06, G(s * 0.075, 0.44, 0.80), "bone", r2=0.002, rot=(0, s * -30, 0), n=6)
    k.tube([G(0, 0.62, -0.55), G(0, 0.64, -0.6), G(0.03, 0.6, -0.62), G(0.0, 0.58, -0.6)], 0.012, hide,
           n=6, r_end=0.006)


@recipe("pig_body", "models/mob/pig_body.glb", kind="asis")
def pig_body(k):
    _pig_body(k)


def _pig_leg(k, at=(0, 0, 0)):
    x, y, z = at
    k.lathe([(0.0, 0.02), (0.075, 0.0), (0.07, -0.08), (0.05, -0.17), (0.04, -0.26), (0.043, -0.29),
             (0.0, -0.29)], G(x, y, z), "pighide", n=10)
    k.cyl(0.042, 0.03, G(x, y - 0.305, z), "hoof", n=10, bevel=0.004)


@recipe("pig_leg", "models/mob/pig_leg.glb", kind="asis", tex=512)
def pig_leg(k):
    _pig_leg(k)


def _wolf_body(k):
    fur = "wolffur"
    k.lathe([(0.0, -0.47), (0.12, -0.45), (0.17, -0.36), (0.18, -0.15), (0.19, 0.08), (0.21, 0.22),
             (0.17, 0.36), (0.0, 0.42)], G(0, 0.62, 0), fur, axis="Y", n=14, scale=(0.95, 1.0, 1.0))
    k.ball(0.19, G(0, 0.68, 0.14), fur, scale=(1.05, 1.3, 0.88), n=12)
    k.tube([G(0, 0.66, 0.30), G(0, 0.65, 0.42), G(0, 0.62, 0.54)], 0.12, fur, n=12, r_end=0.105)
    k.ball(0.11, G(0, 0.60, 0.62), fur, scale=(0.95, 1.1, 0.95), n=12)
    k.lathe([(0.0, 0.62), (0.07, 0.66), (0.06, 0.76), (0.04, 0.86), (0.032, 0.895), (0.0, 0.90)],
            G(0, 0.0, 0.0), fur, axis="Y", n=10, scale=(1.0, 1.0, 0.9)).location = G(0, 0.555, 0)
    k.ball(0.018, G(0, 0.565, 0.895), "eye", n=6)
    for s in (-1, 1):
        k.ball(0.012, G(s * 0.06, 0.63, 0.70), "eye", n=6)
        k.slab([(-0.035, 0.0), (0.035, 0.0), (0.0, 0.11)], 0.014, G(s * 0.065, 0.68, 0.60), fur,
               plane="XZ", rot=(-8, s * -12, 0), bevel=0.0)
    k.tube([G(0, 0.63, -0.42), G(0, 0.58, -0.52), G(0, 0.52, -0.62), G(0, 0.47, -0.72)], 0.05, fur,
           n=8, r_end=0.028)


@recipe("wolf_body", "models/mob/wolf_body.glb", kind="asis")
def wolf_body(k):
    _wolf_body(k)


def _wolf_leg(k, at=(0, 0, 0)):
    x, y, z = at
    # The thigh runs up past the hip into the body: the torso tapers over the
    # rear hips and a leg that stopped at its hip would hang below it.
    k.lathe([(0.0, 0.12), (0.045, 0.11), (0.064, 0.03), (0.052, -0.10), (0.032, -0.22), (0.028, -0.34),
             (0.034, -0.38), (0.0, -0.40)], G(x, y, z), "wolffur", n=10)
    k.ball(0.034, G(x, y - 0.385, z + 0.02), "hoof", scale=(1.0, 1.5, 0.5), n=8)


@recipe("wolf_leg", "models/mob/wolf_leg.glb", kind="asis", tex=512)
def wolf_leg(k):
    _wolf_leg(k)


# A killed animal on its side (wire v84): body and legs at rest, rolled a
# quarter turn about its length and lifted onto the ground by the export.
PIG_HIPS = ((-0.16, 0.32, 0.34), (0.16, 0.32, 0.34), (-0.16, 0.32, -0.34), (0.16, 0.32, -0.34))
WOLF_HIPS = ((-0.14, 0.40, 0.34), (0.14, 0.40, 0.34), (-0.14, 0.40, -0.40), (0.14, 0.40, -0.40))


@recipe("pig_carcass", "models/mob/pig_carcass.glb", kind="held")
def pig_carcass(k):
    _pig_body(k)
    for h in PIG_HIPS:
        _pig_leg(k, h)
    k.turn_all((0, -90, 0))


@recipe("wolf_carcass", "models/mob/wolf_carcass.glb", kind="held")
def wolf_carcass(k):
    _wolf_body(k)
    for h in WOLF_HIPS:
        _wolf_leg(k, h)
    k.turn_all((0, -90, 0))


# ── the ziggurat's keycard crates ───────────────────────────────────────────
# The supply crate's volume (1.1 × 0.8 × 0.8, centred like its massing), as a
# painted steel locker per tier so the tier reads at a glance.

def _tier_crate(k, paint, stripe):
    k.box((1.04, 0.74, 0.58), (0, 0, 0.31), paint, bevel=0.025, segs=2)
    k.box((1.10, 0.80, 0.14), (0, 0, 0.70), paint, bevel=0.03, segs=2)
    k.box((1.08, 0.78, 0.03), (0, 0, 0.615), "dark", bevel=0.004)
    for x in (-0.5, 0.5):
        for y in (-0.35, 0.35):
            k.box((0.08, 0.08, 0.62), (x, y, 0.31), "dark", bevel=0.008)
    for x in (-0.25, 0.0, 0.25):
        k.box((0.05, 0.75, 0.56), (x, 0, 0.30), paint, bevel=0.008)
    k.box((1.105, 0.805, 0.04), (0, 0, 0.70), stripe, bevel=0.004)
    for s in (-1, 1):
        k.torus(0.07, 0.012, (s * 0.54, 0, 0.45), "dark", axis="X", n=14, m=5, arc=180, rot=(0, 0, 0))
    for x in (-0.3, 0.3):
        k.box((0.08, 0.03, 0.10), (x, -0.39, 0.60), "steel", bevel=0.004)
    k.box((0.16, 0.012, 0.10), (0, -0.375, 0.40), "rubber", bevel=0.0)
    k.box((0.10, 0.014, 0.03), (0, -0.378, 0.42), stripe, bevel=0.0)


@recipe("crate_green", "models/prop/crate_green.glb", kind="centre", size=(1.1, 0.8, 0.8))
def crate_green(k):
    _tier_crate(k, "green", "kgreen")


@recipe("crate_blue", "models/prop/crate_blue.glb", kind="centre", size=(1.1, 0.8, 0.8))
def crate_blue(k):
    _tier_crate(k, "blue", "white")


@recipe("crate_elite", "models/prop/crate_elite.glb", kind="centre", size=(1.1, 0.8, 0.8))
def crate_elite(k):
    _tier_crate(k, "red", "yellow")
