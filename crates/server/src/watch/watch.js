"use strict";
const byId = id => document.getElementById(id);
const picture = byId("view"), badge = byId("connection"), events = byId("events");
const POLL_MS = 250, STALE_MS = 5000, REQUEST_MS = 2000, EVENT_CAP = 6;
let previous = null, shown = null, freshAt = 0, stopped = false, imageUrl = null;
const clock = seconds => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
function connection(label, live = false) { badge.textContent = label; badge.dataset.live = String(live); }
function note(text) {
  const li = document.createElement("li"); li.textContent = text; events.prepend(li);
  while (events.children.length > EVENT_CAP) events.lastElementChild.remove();
}
function present(state) {
  if (!previous || state.seconds < previous.seconds) events.replaceChildren();
  if (!previous || state.action !== previous.action) note(state.action);
  if (previous && state.wood > previous.wood) note(`+${state.wood - previous.wood} wood in the pack`);
  if (previous && state.hp < previous.hp) note(`Health ${previous.hp} → ${state.hp}`);
  byId("action").textContent = state.action;
  byId("mode").textContent = state.mode.toUpperCase();
  byId("wood").textContent = state.wood.toLocaleString();
  byId("trees").textContent = state.trees.toLocaleString();
  byId("time").textContent = clock(state.seconds);
  byId("hp").textContent = `${state.hp} / ${state.hp_max}`;
  byId("health").max = state.hp_max || 1; byId("health").value = state.hp;
  byId("waiting").hidden = true; picture.hidden = false;
  previous = state; shown = state.frame; freshAt = performance.now() - state.age_ms;
  connection("LIVE", true);
}
async function poll() {
  const abort = new AbortController(), timeout = setTimeout(() => abort.abort(), REQUEST_MS);
  try {
    const requestedAt = performance.now();
    const response = await fetch("state.json", { cache: "no-store", signal: abort.signal });
    if (!response.ok) throw new Error("unavailable");
    const state = await response.json();
    if (!state.ready) connection("WAITING FOR CAMERA");
    else if (state.age_ms > STALE_MS) connection("FEED STALLED");
    else if (state.frame !== shown) {
      const frame = await fetch(`frame.jpg?n=${state.frame}`, { cache: "no-store", signal: abort.signal });
      if (!frame.ok) throw new Error("frame changed");
      const blob = await frame.blob(), oldUrl = imageUrl;
      imageUrl = URL.createObjectURL(blob);
      picture.src = imageUrl;
      try { await picture.decode(); } finally { if (oldUrl) URL.revokeObjectURL(oldUrl); }
      state.age_ms += performance.now() - requestedAt;
      present(state);
    }
  } catch { connection("FEED OFFLINE"); }
  finally {
    clearTimeout(timeout);
    if (shown !== null && performance.now() - freshAt > STALE_MS) connection("FEED STALLED");
    if (!stopped) setTimeout(poll, POLL_MS);
  }
}
window.addEventListener("pagehide", () => { stopped = true; if (imageUrl) URL.revokeObjectURL(imageUrl); });
poll();
