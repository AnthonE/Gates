# assets/skins — painted skins

A painted skin is one square PNG drawn on an item's canvas (`skinkit template
<item>`), listed in `content/skins.toml` by its sha256 (`texture`). The game
fetches it from the platform by that hash, so this folder is the source a
skin is reviewed and kept from; the platform's copy is a mirror
(`data/items/skins/<sha256>.png` in the platform repo).

Same licence rail as every asset here (`ART.md` §7): **CC0 or your own work,
CC-BY with a `NOTICE` entry; NC and SA refused** (the game is sold), and never
another game's art. A row is required: a PNG with no row does not ship.

| file | item | maker | licence | sha256 |
|---|---|---|---|---|
| `field_medic_tunic.png` | Burlap Tunic | drawn by `draw_field_medic.py` (Claude, for the house) | CC0 | `a3e59a08d11ce4f21077c189df6d489e813f1b44ec14f25fa1ba74e84c980b58` |
| `patchwork_tunic.png` | Burlap Tunic | drawn by `draw_patchwork_tunic.py` (Claude, for the house) | CC0 | `4309c0d49f73af753f1cd1631051018ff25df21c3ccfd3412e9b3127c06c40d9` |
| `pill_tunic.png` | Burlap Tunic | the platform's `deploy/solana-items/art/pill-skin.png`, drawn by `watchtower/art/make_pill.py` (Claude, for the house) | ours; a remix of pump.fun's pill, the operator's call (`SOLANA-ITEMS.md` §1) | `29cc12b676c2f3ffad3c29b9023c35ced65dba03b1f48a1cb3c8f6cb2ab9394c` |
