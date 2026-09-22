# Visible loot and shared item models, 2026-09-22

Ground-item sync messages set `APPLIED2_GITEMS`; `structures_changed` watched
only the first applied word. After the kit was built, a drop or pickup alone
could not reconcile the drawing. A building or backpack update could make
the missing loot appear incidentally.

The renderer now watches the second word, including empty resets. Loose
stacks use the existing held mesh and material where available. A tied pouch
replaces the cube for unsupported items and stays visible while a model loads;
late catalog entries and completed loads replace it without another drop.
Known item names are cached, and idle frames do not reconcile the store.

Ground poses use transformed mesh bounds, the local slope and the near
terrain's actual triangle heights. Pickup still uses the server's position.
The generated torch has a tapered shaft and layered wrapping; the revolver
has a round cylinder, recessed muzzle, shaped grip and open trigger guard.
Their existing heights, grips, materials and light behavior are unchanged.

The runtime regression drives real `GItemSync` bytes through `ClientCore`
and a Bevy app with a warmed kit. It covers two drops, taking one, clearing
the last, late names and loading models. Removing the second-word condition
fails with **0 drawn stacks versus 2 expected**. Geometry checks compare
placement against slopes and the triangles emitted by `heightfield`.

Model review uses an isolated Bevy scene under Xvfb/lavapipe, with upright
and grounded torch/revolver models, the textured rock and hatchet, and the
pouch. The local capture and
temporary harness are under `/tmp/gates-loot-review/`. This is a model review,
not a claim that a person has played the scene on a hardware GPU.

Resource items without held models still use the pouch. Drop animation and
physics remain outside this change; no server, protocol or balance changes
are needed. Publishing the client remains a separate act.
