/* companion.js — the page half of crates/server/src/companion.rs.
   Sign in with the wallet you play with (one personal_sign over the shard's
   own challenge), keep the token, and read api/team every few seconds.
   Every string from the API is set as text, never as HTML. */
"use strict";
(() => {
  const KEY = "gates.team.token";
  const POLL_MS = 3000;
  const $ = (id) => document.getElementById(id);
  const kept = {
    get() { try { return localStorage.getItem(KEY) || ""; } catch { return ""; } },
    set(v) { try { if (v) localStorage.setItem(KEY, v); else localStorage.removeItem(KEY); } catch { /* private mode */ } },
  };
  let token = kept.get();
  let timer = null;
  let lastAlert = 0;
  let wallet = "";

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text != null) n.textContent = text;
    return n;
  };
  const short = (a) => a ? a.slice(0, 6) + "…" + a.slice(-4) : "a guest";
  const when = (unix) => {
    const d = Math.max(0, Math.round(Date.now() / 1000 - unix));
    if (d < 60) return "now";
    if (d < 3600) return Math.floor(d / 60) + " min ago";
    if (d < 86400) return Math.floor(d / 3600) + " h ago";
    return Math.floor(d / 86400) + " d ago";
  };
  const hex = (text) => "0x" + Array.from(new TextEncoder().encode(text),
    (b) => b.toString(16).padStart(2, "0")).join("");

  async function api(method, path, body) {
    const headers = {};
    if (token) headers.Authorization = "Bearer " + token;
    if (body) headers["Content-Type"] = "application/json";
    const r = await fetch(path, { method, headers, body: body ? JSON.stringify(body) : undefined, cache: "no-store" });
    let data = null;
    try { data = await r.json(); } catch { /* 202 and 204 carry no body */ }
    return { status: r.status, data };
  }

  async function signIn() {
    const why = $("out-why");
    const eth = window.eloEthereum || window.ethereum;
    if (!eth) {
      why.textContent = "No wallet found in this browser. Open this page in your wallet app's browser, or install a wallet extension.";
      return;
    }
    $("signin").disabled = true;
    why.textContent = "Asking your wallet…";
    try {
      const [address] = await eth.request({ method: "eth_requestAccounts" });
      const q = "address=" + encodeURIComponent(address) + "&domain=" + encodeURIComponent(location.hostname);
      const c = await api("GET", "api/challenge?" + q);
      if (c.status !== 200) throw new Error((c.data && c.data.error) || "no challenge");
      why.textContent = "Sign the message in your wallet. It moves no funds.";
      const signature = await eth.request({ method: "personal_sign", params: [hex(c.data.message), address] });
      const r = await api("POST", "api/login", { address, nonce: c.data.nonce, issued_at: c.data.issued_at, signature });
      if (r.status === 403) throw new Error("this wallet has no character on the island yet. Play once with it, then sign in here");
      if (r.status !== 200) throw new Error((r.data && r.data.error) || "sign-in refused");
      token = r.data.token;
      kept.set(token);
      why.textContent = "";
      show();
    } catch (e) {
      why.textContent = "Not signed in: " + (e && e.message ? e.message : e);
    } finally {
      $("signin").disabled = false;
    }
  }

  function signOut() {
    if (token) api("POST", "api/logout").catch(() => {});
    token = "";
    kept.set("");
    show();
  }

  function show() {
    const on = !!token;
    $("out").hidden = on;
    $("in").hidden = !on;
    clearTimeout(timer);
    if (on) poll();
  }

  async function poll() {
    try {
      const r = await api("GET", "api/team");
      if (r.status === 401) { signOut(); return; }
      if (r.status === 200) paint(r.data);
    } catch { /* the next poll tries again */ }
    timer = setTimeout(poll, POLL_MS);
  }

  function mark(parent, u, v, cls, label) {
    const n = el("div", cls);
    n.style.left = (u * 100) + "%";
    n.style.top = (v * 100) + "%";
    if (label) n.appendChild(el("span", "", label));
    parent.appendChild(n);
  }

  function paint(d) {
    wallet = d.wallet;
    $("clock").textContent = d.night ? "NIGHT ON THE ISLAND" : "DAY ON THE ISLAND";
    $("online").textContent = d.players + " ONLINE";
    $("me-name").textContent = d.me && d.me.name ? d.me.name.toUpperCase() : short(d.wallet);

    const note = $("note");
    if (!d.me) {
      note.textContent = "You're not on the island yet. Play once with this wallet and you'll show up here.";
    } else if (d.team.length < 2) {
      note.textContent = "You're not on a team. Team up in the game to see each other here and use team chat.";
    } else {
      note.textContent = "";
    }
    note.hidden = !note.textContent;

    const now = Date.now() / 1000;
    const hot = d.alerts.filter((a) => now - a.at < 600);
    const alert = $("alert");
    alert.hidden = !hot.length;
    if (hot.length) alert.textContent = "Your base at " + (hot[0].grid || "?") + " is taking damage (" + when(hot[0].at) + ").";

    const marks = $("marks");
    marks.replaceChildren();
    for (const b of d.bases) {
      const hit = hot.some((a) => a.x === b.x && a.z === b.z);
      mark(marks, b.u, b.v, "base" + (hit ? " hit" : ""), "");
    }
    for (const p of d.team) {
      const me = p.wallet === d.wallet;
      mark(marks, p.u, p.v, "mark " + p.state + (me ? " me" : ""), me ? "you" : (p.name || short(p.wallet)));
    }

    const team = $("team");
    team.replaceChildren();
    for (const p of d.team) {
      const li = el("li");
      li.appendChild(el("span", "dot " + p.state));
      li.appendChild(el("span", "", (p.name || short(p.wallet)) + (p.leader ? " ★" : "")));
      li.appendChild(el("small", "", (p.online ? "" : "offline · ") + p.state + " · " + (p.grid || "—")));
      team.appendChild(li);
    }
    if (!d.team.length) team.appendChild(el("li", "quiet", "Nobody yet."));

    const chat = $("chat");
    const atBottom = chat.scrollTop + chat.clientHeight >= chat.scrollHeight - 4;
    chat.replaceChildren();
    for (const l of d.chat) {
      const li = el("li");
      li.appendChild(el("b", "", (l.name || short(l.wallet)) + ": "));
      li.appendChild(document.createTextNode(l.text));
      li.appendChild(el("small", "", when(l.at) + (l.web ? " · from here" : "")));
      chat.appendChild(li);
    }
    if (!d.chat.length) chat.appendChild(el("li", "quiet", "No team chat yet."));
    if (atBottom) chat.scrollTop = chat.scrollHeight;

    const alerts = $("alerts");
    alerts.replaceChildren();
    for (const a of d.alerts) alerts.appendChild(el("li", "", "Base at " + (a.grid || "?") + " took damage · " + when(a.at)));
    if (!d.alerts.length) alerts.appendChild(el("li", "quiet", "None. Quiet is good."));

    const newest = d.alerts.length ? d.alerts[0].at : 0;
    if (lastAlert && newest > lastAlert && window.Notification && Notification.permission === "granted") {
      try { new Notification("Gates: your base is taking damage", { body: "Base at " + (d.alerts[0].grid || "?") }); } catch { /* not allowed here */ }
    }
    lastAlert = Math.max(lastAlert, newest);
  }

  async function say(e) {
    e.preventDefault();
    const input = $("text");
    const text = input.value.trim();
    if (!text) return;
    const r = await api("POST", "api/chat", { text }).catch(() => ({ status: 0 }));
    const why = $("say-why");
    if (r.status === 202) {
      input.value = "";
      why.textContent = "";
      clearTimeout(timer);
      setTimeout(poll, 400);
    } else {
      why.textContent = ({ 409: "You need a team to use team chat.", 429: "Slow down a little.", 400: "That line can't be sent." })[r.status] || "Not sent. Try again.";
    }
  }

  $("signin").addEventListener("click", signIn);
  $("signout").addEventListener("click", signOut);
  $("say").addEventListener("submit", say);
  $("notify").addEventListener("click", () => {
    if (window.Notification) Notification.requestPermission();
  });
  show();
})();
