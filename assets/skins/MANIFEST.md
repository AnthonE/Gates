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
| `field_medic_tunic.png` | Burlap Tunic | drawn by `draw_field_medic.py` (Claude, for the house) | CC0 | `07071a958a3e81818571e2b088bca08b6d865053dcbe346e4c89a9396109bf99` |
