"""Reference values for the UIParticle canvas bake (ui_particle/bake.rs).

An independent float64 transcription of the source's C# order, on sampled
synthetic inputs:

- `UIParticleUpdater.ModifyScale`: the UIParticle node's local scale is the
  per-component inverse of the root canvas' local scale (1 where that is
  approximately 0), kept as it is when already within `Mathf.Approximately`
  of it on the squared distance.
- `BakingCamera.GetCamera`: orthographicSize = max(rect w, rect h) *
  rootCanvas.scaleFactor.
- `UIParticleUpdater.BakeMesh`: rootMatrix = Rotate(root.rotation).inverse *
  Scale(root.lossyScale).inverse; a system below the root simulating in Local
  space takes Translate(root.InverseTransformPoint(ps.position)) * rootMatrix,
  one in World space rootMatrix * Translate(-root.position); a system on the
  root takes GetScaledMatrix (Local: Rotate(rotation).inverse *
  Scale(lossyScale).inverse; World: worldToLocalMatrix); then
  Scale(scale) * matrix with scale = rootCanvas.localScale (x) scale3D while
  the canvas scaler is ignored, scale3D otherwise. The CanvasRenderer draws
  the result in the UIParticle node's space; the value compared is a baked
  point carried into the root canvas' local space.
- The extra world simulation: diff * (1 - 1 / max(0.001, scale)).

Transform semantics follow the engine: localToWorld is the product of the
local TRS matrices from the root, rotation the product of the local
rotations, lossyScale the diagonal of inverse(rotation) times the world
rotation-and-scale matrix, InverseTransformPoint applied level by level
from the root (subtract the local position, rotate by the inverse local
rotation, divide by the local scale).

Run: python ui_particle_bake.py > ui-particle-bake.json
"""

import json
import math
import random

import numpy as np

EPSILON = 1.401298464324817e-45  # Mathf.Epsilon (the smallest subnormal float)


def approximately(a, b):
    return abs(b - a) < max(1e-6 * max(abs(a), abs(b)), EPSILON * 8)


def f32(x):
    return float(np.float32(x))


def quat_mul(a, b):
    ax, ay, az, aw = a
    bx, by, bz, bw = b
    return (
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by + ay * bw + az * bx - ax * bz,
        aw * bz + az * bw + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    )


def quat_inverse(q):
    x, y, z, w = q
    n = x * x + y * y + z * z + w * w
    return (-x / n, -y / n, -z / n, w / n)


def quat_mat3(q):
    x, y, z, w = q
    return np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])


def rotate(q):
    m = np.eye(4)
    m[:3, :3] = quat_mat3(q)
    return m


def scale(v):
    return np.diag([v[0], v[1], v[2], 1.0])


def translate(v):
    m = np.eye(4)
    m[:3, 3] = v
    return m


def trs(p, q, s):
    return translate(p) @ rotate(q) @ scale(s)


class Transform:
    def __init__(self, parent, position, rotation, local_scale):
        self.parent = parent
        self.local_position = np.array(position, dtype=float)
        self.local_rotation = rotation
        self.local_scale = np.array(local_scale, dtype=float)

    @property
    def local_to_world(self):
        m = trs(self.local_position, self.local_rotation, self.local_scale)
        return m if self.parent is None else self.parent.local_to_world @ m

    @property
    def rotation(self):
        if self.parent is None:
            return self.local_rotation
        return quat_mul(self.parent.rotation, self.local_rotation)

    @property
    def position(self):
        return self.local_to_world[:3, 3]

    @property
    def lossy_scale(self):
        m = quat_mat3(quat_inverse(self.rotation)) @ self.local_to_world[:3, :3]
        return np.array([m[0, 0], m[1, 1], m[2, 2]])

    @property
    def world_to_local(self):
        return np.linalg.inv(self.local_to_world)

    def inverse_transform_point(self, p):
        local = p if self.parent is None else self.parent.inverse_transform_point(p)
        local = local - self.local_position
        local = quat_mat3(quat_inverse(self.local_rotation)) @ local
        return np.array([0.0 if s == 0 else v / s for v, s in zip(local, self.local_scale)])


def modify_scale(current, canvas_scale):
    target = np.array([1.0 if approximately(s, 0.0) else 1.0 / s for s in canvas_scale])
    d = np.asarray(current) - target
    return np.asarray(current) if approximately(float(d @ d), 0.0) else target


def bake_matrix(root, system, space, bake_scale):
    root_matrix = np.linalg.inv(rotate(root.rotation)) @ np.linalg.inv(scale(root.lossy_scale))
    if system is not root:
        if space == "Local":
            matrix = translate(root.inverse_transform_point(system.position)) @ root_matrix
        else:
            matrix = root_matrix @ translate(-root.position)
    elif space == "Local":
        matrix = np.linalg.inv(rotate(system.rotation)) @ np.linalg.inv(scale(system.lossy_scale))
    else:
        matrix = system.world_to_local
    return scale(bake_scale) @ matrix


def rand_quat(rng, planar=False):
    if planar:
        a = rng.uniform(-math.pi, math.pi)
        q = (0.0, 0.0, math.sin(a / 2), math.cos(a / 2))
    else:
        q = [rng.gauss(0, 1) for _ in range(4)]
    n = math.sqrt(sum(c * c for c in q))
    return tuple(f32(c / n) for c in q)


def rand_vec(rng, lo, hi, z=True):
    return [f32(rng.uniform(lo, hi)), f32(rng.uniform(lo, hi)), f32(rng.uniform(lo, hi)) if z else 0.0]


def main():
    rng = random.Random(0x75697061)
    out = {"modifyScale": [], "orthographicSize": [], "worldDisplacement": [], "bake": []}

    for i in range(64):
        c = [f32(rng.uniform(0.001, 0.05)) for _ in range(3)]
        if i % 8 == 0:
            c[i % 3] = 0.0
        if i % 4 == 1:
            # A scale within Approximately of the target on the squared
            # distance is kept; with a huge canvas scale the target is tiny
            # and a distinct current scale falls inside it.
            c = [f32(rng.uniform(1e25, 1e30)) for _ in range(3)]
            current = [f32(1.5 / v) for v in c]
        elif i % 4 == 2:
            current = [f32(rng.uniform(0.5, 2.0)) for _ in range(3)]
        else:
            current = [0.0, 0.0, 0.0]
        out["modifyScale"].append({"current": current, "canvasScale": c,
                                   "expected": [f32(v) for v in modify_scale(current, c)]})

    for _ in range(32):
        w, h = f32(rng.uniform(600, 3000)), f32(rng.uniform(600, 3000))
        sf = f32(rng.uniform(0.2, 3.0))
        out["orthographicSize"].append({"rect": [w, h], "scaleFactor": sf, "expected": f32(max(w, h) * sf)})

    for i in range(32):
        position = rand_vec(rng, -50, 50)
        cached = rand_vec(rng, -50, 50)
        s = [f32(rng.uniform(0.0005, 3.0)) for _ in range(3)]
        drawn = i % 5 != 0
        diff = (np.array(position) - np.array(cached)) * np.array([1 - 1 / max(0.001, v) for v in s])
        out["worldDisplacement"].append({"position": position, "cached": cached, "scale": s,
                                         "drawnBefore": drawn,
                                         "expected": [f32(v) for v in (diff if drawn else np.zeros(3))]})

    for i in range(256):
        c = f32(rng.uniform(0.002, 0.05))
        canvas = Transform(None, rand_vec(rng, -10, 10), rand_quat(rng, planar=i % 2 == 0), [c, c, c])
        node = canvas
        links = []
        for _ in range(rng.randint(0, 3)):
            link = {"position": rand_vec(rng, -300, 300, z=False), "rotation": rand_quat(rng, planar=i % 3 != 0),
                    "scale": [f32(rng.uniform(0.4, 1.6)) for _ in range(3)]}
            links.append(link)
            node = Transform(node, link["position"], link["rotation"], link["scale"])
        ignore = i % 7 != 3
        serialized = [0.0, 0.0, 0.0] if ignore else [f32(rng.uniform(0.5, 2.0)) for _ in range(3)]
        own = modify_scale(serialized, [c, c, c]) if ignore else np.array(serialized)
        ui = {"position": rand_vec(rng, -200, 200, z=False), "rotation": rand_quat(rng, planar=i % 4 != 0),
              "serializedScale": serialized}
        root = Transform(node, ui["position"], ui["rotation"], [f32(v) for v in own])
        system = root
        system_links = []
        for _ in range(rng.choice([0, 1, 1, 2])):
            link = {"position": rand_vec(rng, -3, 3), "rotation": rand_quat(rng),
                    "scale": [f32(rng.uniform(0.5, 1.5)) for _ in range(3)]}
            system_links.append(link)
            system = Transform(system, link["position"], link["rotation"], link["scale"])
        space = "Local" if i % 3 != 2 else "World"
        scale3d = [f32(rng.uniform(10, 120)) for _ in range(3)]
        bake_scale = np.array([c, c, c]) * np.array(scale3d) if ignore else np.array(scale3d)
        point = rand_vec(rng, -5, 5)
        to_canvas = canvas.world_to_local @ root.local_to_world @ bake_matrix(root, system, space, bake_scale)
        v = to_canvas @ np.array(point + [1.0])
        out["bake"].append({
            "canvas": {"position": canvas.local_position.tolist(), "rotation": list(canvas.local_rotation),
                       "scale": c},
            "links": links, "uiParticle": ui, "ignoreCanvasScaler": ignore, "systemLinks": system_links,
            "space": space, "scale3d": scale3d, "point": point, "expected": v[:3].tolist(),
        })

    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
