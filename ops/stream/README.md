# Gates stream overlays

Edit `config.js` (today's agenda, countdown minutes, ticker lines, socials), save,
then right-click the source in OBS -> Refresh.

| OBS source | URL (browser source URL field, 1920x1080) |
|---|---|
| Card - Starting | file:///home/anthony/stream-overlays/card.html?m=start  (add &min=10 or &until=19:30) |
| Card - BRB | file:///home/anthony/stream-overlays/card.html?m=brb |
| Card - Ending | file:///home/anthony/stream-overlays/card.html?m=end |
| Overlay - URL Bug | file:///home/anthony/stream-overlays/bug.html  (?ticker=0 hides the ticker) |
| Cam frame (new) | file:///home/anthony/stream-overlays/frame.html?label=CAM&name=anthony — size it to the cam, place above it |
| Chat frame (new) | file:///home/anthony/stream-overlays/frame.html?label=COMMS&name=chat&panel=1 — place below the chat source |
| Alert frame (new) | file:///home/anthony/stream-overlays/frame.html?label=INCOMING&name=alerts |

Prices: DexScreener public API, refreshed every 60 s (ELO/ETH and JUNK/ELO on Robinhood Chain).

## OBS scene collection

`obs/Gates.json` is the `Gates` scene collection (Starting Soon / Game / BRB /
Ending). Its browser sources point at `file:///home/anthony/stream-overlays/…`;
on another machine, import it (Scene Collection -> Import) and fix those URLs
to wherever this folder lives. Profiles (stream keys) are deliberately not here.
