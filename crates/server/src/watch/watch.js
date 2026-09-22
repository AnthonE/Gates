"use strict";
const byId = id => document.getElementById(id);
const picture = byId("view"), badge = byId("connection"), events = byId("events");
const POLL_MS = 250, STALE_MS = 5000, REQUEST_MS = 2000, EVENT_CAP = 6;
let previous = null, shown = null, freshAt = 0, stopped = false, imageUrl = null;
const clock = seconds => `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
// Goal labels are the controller's own vocabulary: `gather_wood`,
// `craft:Stone Hatchet`. Shown as words; always set as text, never markup.
function goalText(label) {
  if (!label) return "Choosing the next goal";
  if (label.startsWith("craft:")) return `Craft a ${label.slice(6)}`;
  const words = label.replace(/_/g, " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}
function meter(pair) { return Array.isArray(pair) ? `${pair[0]} / ${pair[1]}` : "—"; }
function connection(label, live = false) { badge.textContent = label; badge.dataset.live = String(live); }
function note(text) {
  const li = document.createElement("li"); li.textContent = text; events.prepend(li);
  while (events.children.length > EVENT_CAP) events.lastElementChild.remove();
}
function fill(list, rows, empty) {
  const items = rows.length ? rows : [empty];
  list.replaceChildren(...items.map(text => { const li = document.createElement("li"); li.textContent = text; return li; }));
}
function present(state) {
  if (!previous || state.seconds < previous.seconds) events.replaceChildren();
  if (!previous || state.goal !== previous.goal) note(state.goal ? `New goal: ${goalText(state.goal)}` : state.action);
  if (previous && state.deaths > previous.deaths) note("Died");
  if (previous && state.respawns > previous.respawns) note("Answered the death screen: back on a beach");
  if (previous && state.hp < previous.hp) note(`Health ${previous.hp} → ${state.hp}`);
  if (previous && state.paused && !previous.paused) note("Model decisions paused: spend cap");
  byId("action").textContent = state.action;
  byId("mode").textContent = state.mode.toUpperCase();
  byId("mode").dataset.paused = String(!!state.paused);
  byId("goal").textContent = goalText(state.goal);
  byId("reason").textContent = state.reason || "No reason given yet.";
  byId("food").textContent = meter(state.food);
  byId("water").textContent = meter(state.water);
  byId("lives").textContent = `${state.deaths} · ${state.respawns}`;
  byId("time").textContent = clock(state.seconds);
  byId("hp").textContent = `${state.hp} / ${state.hp_max}`;
  byId("health").max = state.hp_max || 1; byId("health").value = state.hp;
  fill(byId("pack"), (state.inventory || []).map(i => `${i.name} × ${i.count.toLocaleString()}`), "Empty");
  fill(byId("history"), (state.history || []).map(h =>
    `${goalText(h.goal)}: ${h.outcome}${h.why ? ` (${h.why})` : ""}${h.gained ? `, +${h.gained}` : ""}`), "None yet.");
  // A spectator seat on the bot's own shard (NETCODE.md §2.3), linked only
  // when the operator named a Gates web page. Scoped as the server says:
  // a loopback shard is this machine only. http(s) links only.
  const link = state.spectate, box = byId("spectate");
  if (link && /^https?:\/\//.test(link.url)) {
    const a = byId("spectate-link");
    if (a.getAttribute("href") !== link.url) a.setAttribute("href", link.url);
    byId("spectate-scope").textContent = link.scope;
    box.hidden = false;
  } else box.hidden = true;
  byId("waiting").hidden = true; picture.hidden = false;
  previous = state; shown = state.frame; freshAt = performance.now() - state.age_ms;
  connection("LIVE", true);
}
async function poll() {
  const abort = new AbortController(), timeout = setTimeout(() => abort.abort(), REQUEST_MS);
  try {
    const requestedAt = performance.now();
    const response = await fetch("frame.jpg", { cache: "no-store", signal: abort.signal });
    if (response.status === 503) {
      await response.body?.cancel();
      connection("WAITING FOR CAMERA");
      return;
    }
    if (!response.ok) throw new Error("unavailable");
    // The header belongs to this exact JPEG. No second round trip can race
    // the next capture, even when a visitor's connection is slower than it.
    const state = JSON.parse(response.headers.get("X-Bot-State"));
    const blob = await response.blob();
    const receivedAt = performance.now();
    state.age_ms += receivedAt - requestedAt;
    if (state.age_ms > STALE_MS) connection("FEED STALLED");
    else if (state.frame !== shown || state.seconds < previous.seconds) {
      const oldUrl = imageUrl;
      imageUrl = URL.createObjectURL(blob);
      picture.src = imageUrl;
      try { await picture.decode(); } finally { if (oldUrl) URL.revokeObjectURL(oldUrl); }
      state.age_ms += performance.now() - receivedAt;
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
