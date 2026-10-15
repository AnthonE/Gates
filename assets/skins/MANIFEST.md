# assets/skins — painted skins

A painted skin is one square PNG drawn on an item's canvas (`skinkit template
<item>`), listed in `content/skins.toml` by its sha256 (`texture`). The game
fetches it from the platform by that hash, so this folder is the source a
skin is reviewed and kept from; the platform's copy is a mirror
(`scry-forge/data/items/skins/<sha256>.png`).

Same licence rail as every asset here (`ART.md` §7): **CC0 or your own work,
CC-BY with a `NOTICE` entry; NC and SA refused** (the game is sold), and never
another game's art. A row is required: a PNG with no row does not ship.

| file | item | maker | licence | sha256 |
|---|---|---|---|---|
| `field_medic_tunic.png` | Burlap Tunic | drawn by `draw_field_medic.py` (Claude, for the house) | CC0 | `a3e59a08d11ce4f21077c189df6d489e813f1b44ec14f25fa1ba74e84c980b58` |
