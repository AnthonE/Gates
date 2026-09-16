# Road material shortlist — 2026-09-16

Research for a later surface pass. **No textures shipped in this change.**
Provider metadata and downloaded 1K albedos were inspected; these are visual
observations, not an in-game comparison. Candidates stay outside the repository
in `/tmp/gates-road-materials`.

| Candidate | Physical scale | Maps available | Four runtime maps, original 1K JPEG bytes |
|---|---|---|---|
| [Poly Haven Asphalt 02](https://polyhaven.com/a/asphalt_02) | 3 × 3 m | Diffuse, NormalGL, roughness, AO, height | 3,049,197 |
| [Poly Haven Aerial Asphalt 01](https://polyhaven.com/a/aerial_asphalt_01) | 30 × 30 m | Same five | 1,945,830 |
| [Poly Haven Asphalt 03](https://polyhaven.com/a/asphalt_03) | 2.05 × 2.05 m; page rounds to 2 m | Same five | 3,418,399 |

Dimensions come from the provider's millimetre metadata:
[02](https://api.polyhaven.com/info/asphalt_02),
[aerial](https://api.polyhaven.com/info/aerial_asphalt_01),
[03](https://api.polyhaven.com/info/asphalt_03). Costs sum diffuse, normal,
roughness and AO from the corresponding `/files/<asset>` endpoints; height is
available but excluded. These are source downloads, before our encoding.

All three downloadable assets are **CC0**, including commercial redistribution,
under [Poly Haven's licence](https://polyhaven.com/license); each asset page
also labels CC0. Website preview renders have a different copyright policy:
ship the texture maps, not the rendered thumbnails. No purchase is needed.

**Recommend Asphalt 02 for the next material comparison.** Its inspected albedo
contains embedded aggregate, worn patches and narrow dark fissures, giving the
pavement detail that tinted gravel lacks. Its strongest cracks run nearly
parallel: plain tiling would repeat them visibly every 3 m. Macro tint alone
will not hide that pattern. Test sparse crack/repair patches or deterministic
phase/rotation variation, keeping normal transforms aligned with the albedo.

Aerial Asphalt 01 is useful as a broader wear layer: long cracks and tire arcs
repeat less frequently at its authored 30 m scale. At 1K it resolves only
about 2.93 cm per texel, so it needs separate near-field grain. Its distinctive
tire arcs can still reveal repetition. Asphalt 03 is the quieter repair-patch
alternative: fine grain and weak staining, brown in the source, without
prominent cracks. Neither is automatically a better full replacement.

Keep pavement separate from `rock`, which also dresses cliffs and props. A
road-only array extension can preserve the four sim splat identities and
existing texture-binding count: append albedo/normal layer 4 and roughness/AO
layers 8/9, retaining current terrain indices. Source dimensions, formats,
samplers and mip chains must match the arrays (`render/textures.rs::stack`).

Four extra 1024² RGBA8 layers with full mip chains calculate to **21.33 MiB**
of GPU storage, plus transient source images while stacking. Keeping the
branch's fine gravel and independently sampling pavement adds four texture
fetches per ground fragment with the present shader structure. **Hardware
performance cost is unmeasured.** Extra layers do not add texture bindings.

Before shipping: measure with the existing material estimators, register the
source scale and mean correction, record manifest provenance, extend array
and channel checks, and inspect walking views on native and WebGL. Treat the
30 m option as a documented macro layer, not a silent change to the ground
base-tile ceiling. Blend fixed-scale samples; interpolating world UV scale
reintroduces the edge distortion found during the first road-surface pass.
