// DOM HUD, updated on a slow timer well outside the RAF path (L8: UI in
// plain DOM outside the loop). Toasts are event-driven — they fire from
// the event-lane pump, which is also not the RAF path.

/** Toast lifetime and cap (cosmetic; the stack reads like the reference
 * gather feedback — stacking "+N Thing" lines that fade). */
const TOAST_MS = 1600;
const TOAST_CAP = 8;

/** Chat log: lines kept on screen, and how long a line stays visible once
 * the composer is closed (cosmetic — the reference's kill/contacts feed
 * fades the same way). Open the composer and the whole log holds. */
const CHAT_CAP = 8;
const CHAT_FADE_MS = 12000;

export class Hud {
  constructor() {
    this.el = document.getElementById("hud");
    this.cross = document.getElementById("cross");
    this.hotbar = document.getElementById("hotbar");
    this.toasts = document.getElementById("toasts");
    this.craft = document.getElementById("craft");
    this.chat = document.getElementById("chat");
    this.chatlog = document.getElementById("chatlog");
    this.chatinput = document.getElementById("chatinput");
    this.chatOpen = false;
    /** Set by main.js: (raw line) → bool sent. */
    this.onChatSend = () => false;
    this.chatinput.addEventListener("keydown", (e) => {
      // The composer swallows every key while it is open, so a "w" is a
      // letter and not a step forward.
      e.stopPropagation();
      if (e.code === "Enter" || e.code === "NumpadEnter") {
        const raw = this.chatinput.value.trim();
        if (raw) this.onChatSend(raw);
        this.closeChat();
      } else if (e.code === "Escape") {
        this.closeChat();
      }
    });
    // The death screen (ALPHA.md §1: "death screen (who/what killed you —
    // range and weapon, no map position), choose beach or a bag"). Plain
    // DOM outside the render loop like the rest of the HUD, and built
    // once at construction — a screen that allocated its own buttons on
    // the frame a player died would do it at the worst possible moment.
    this.death = document.getElementById("death");
    this.deathCause = document.getElementById("deathcause");
    this.deathNote = document.getElementById("deathnote");
    this.respawnBag = document.getElementById("respawnbag");
    this.respawnBeach = document.getElementById("respawnbeach");
    this.deathOpen = false;
    /** Set by main.js: (onBag: bool) → void. */
    this.onRespawn = () => {};
    this.respawnBag.addEventListener("click", () => this.answerDeath(true));
    this.respawnBeach.addEventListener("click", () => this.answerDeath(false));
    this.vitals = document.getElementById("vitals");
    this.vitalsRows = null;
    this.lastVitals = "";
    this.craftq = document.getElementById("craftq");
    this.build = document.getElementById("build");
    this.craftOpen = false;
    this.last = "";
    this.lastBuild = "";
    this.cells = [];
    this.cellDivs = [];
    this.cellTexts = [];
    this.selected = -1;
    for (let i = 0; i < 6; i++) {
      const cell = document.createElement("div");
      cell.className = "hotcell";
      const label = document.createElement("span");
      cell.appendChild(label);
      this.hotbar.appendChild(cell);
      this.cells.push(label);
      this.cellDivs.push(cell);
      this.cellTexts.push("");
    }
  }

  show() {
    this.el.style.display = "block";
    this.cross.style.display = "block";
    this.hotbar.style.display = "flex";
    this.toasts.style.display = "block";
    this.chatlog.style.display = "block";
  }

  /**
   * The vitals stack — health, hydration, calories, in the order the
   * reference frames stack them bottom-right.
   *
   * A row whose max is 0 is not drawn: the server has stated nothing about
   * that meter, which is what a shard with combat disarmed says about
   * health and a shard with no `[survival]` section says about the pair.
   * That is deliberately different from a meter at 0/100, which is drawn
   * loudly — "no reading" and "empty" are opposite facts and a bar that
   * rendered them the same would be lying at the worst moment.
   *
   * Slow-timer only, and only a changed reading touches the DOM.
   */
  setVitals(hp, max, food, maxFood, water, maxWater) {
    const key = `${max === 0 ? "" : `${hp}/${max}`}|${
      maxWater === 0 ? "" : `${water}/${maxWater}`
    }|${maxFood === 0 ? "" : `${food}/${maxFood}`}`;
    if (key === this.lastVitals) return;
    this.lastVitals = key;
    if (key === "||") {
      this.vitals.style.display = "none";
      return;
    }
    if (!this.vitalsRows) {
      this.vitalsRows = ["", "water", "food"].map((kind) => {
        const row = document.createElement("div");
        row.className = "vrow";
        const bar = document.createElement("div");
        bar.className = "vbar";
        const fill = document.createElement("div");
        fill.className = kind ? `vfill ${kind}` : "vfill";
        bar.appendChild(fill);
        const num = document.createElement("span");
        num.className = "vnum";
        row.appendChild(bar);
        row.appendChild(num);
        this.vitals.appendChild(row);
        return { row, fill, num };
      });
    }
    this.vitals.style.display = "block";
    const rows = [
      [hp, max],
      [water, maxWater],
      [food, maxFood],
    ];
    for (let i = 0; i < 3; i++) {
      const [v, m] = rows[i];
      const r = this.vitalsRows[i];
      if (m === 0) {
        r.row.style.display = "none";
        continue;
      }
      r.row.style.display = "flex";
      r.row.classList.toggle("empty", v === 0);
      r.fill.style.width = `${Math.max(0, Math.min(100, (v / m) * 100))}%`;
      r.num.textContent = String(v);
    }
  }

  /** Toggle the craft panel; returns whether it is now open. */
  toggleCraft() {
    this.craftOpen = !this.craftOpen;
    this.craft.style.display = this.craftOpen ? "flex" : "none";
    return this.craftOpen;
  }

  /**
   * Rebuild the craft panel. `rows` is the recipe list precomputed by
   * main.js: { recipe, name, count, seconds, gated, gateText, craftable,
   * inputs: [{ text, ok }] }. Event-driven + slow-timer only — never the
   * RAF path.
   */
  setCraft(rows, onCraft) {
    const panel = this.craft;
    panel.textContent = "";
    const h = document.createElement("h2");
    h.textContent = "CRAFT";
    panel.appendChild(h);
    for (const r of rows) {
      const div = document.createElement("div");
      div.className = r.gated ? "crow gated" : "crow";
      const name = document.createElement("div");
      name.className = "cname";
      name.textContent = r.count > 1 ? `${r.name} ×${r.count}` : r.name;
      div.appendChild(name);
      const meta = document.createElement("div");
      meta.className = "cmeta";
      meta.textContent = `${r.seconds}s · `;
      for (let i = 0; i < r.inputs.length; i++) {
        const inp = r.inputs[i];
        const span = document.createElement("span");
        span.className = inp.ok ? "cin ok" : "cin miss";
        span.textContent = inp.text + (i < r.inputs.length - 1 ? " · " : "");
        meta.appendChild(span);
      }
      if (r.gated) {
        const gate = document.createElement("span");
        gate.className = "gate";
        gate.textContent = ` · ${r.gateText}`;
        meta.appendChild(gate);
      }
      div.appendChild(meta);
      if (!r.gated) {
        div.addEventListener("click", (e) => onCraft(r.recipe, e.shiftKey ? 5 : 1));
      }
      panel.appendChild(div);
    }
  }

  /**
   * Rebuild the craft queue strip. `jobs` is [{ index, label }]; empty
   * hides the strip. Clicking a job cancels it.
   */
  setCraftQueue(jobs, onCancel) {
    const strip = this.craftq;
    strip.textContent = "";
    strip.style.display = jobs.length ? "flex" : "none";
    for (const j of jobs) {
      const cell = document.createElement("div");
      cell.className = "qcell";
      cell.textContent = j.label;
      cell.title = "click to cancel";
      cell.addEventListener("click", () => onCancel(j.index));
      strip.appendChild(cell);
    }
  }

  set(text) {
    if (text !== this.last) {
      this.last = text;
      this.el.textContent = text;
    }
  }

  /** The build-mode strip; an empty string hides it. */
  setBuild(text) {
    if (text === this.lastBuild) return;
    this.lastBuild = text;
    this.build.textContent = text;
    this.build.style.display = text ? "block" : "none";
  }

  /** Six strings, one per hotbar slot; only changed cells touch the DOM. */
  setHotbar(texts) {
    for (let i = 0; i < 6; i++) {
      const t = texts[i] || "";
      if (t !== this.cellTexts[i]) {
        this.cellTexts[i] = t;
        this.cells[i].textContent = t;
      }
    }
  }

  /** Highlight the selected hotbar cell; only a change touches the DOM. */
  setSelected(sel) {
    if (sel === this.selected) return;
    if (this.selected >= 0) this.cellDivs[this.selected].classList.remove("sel");
    this.selected = sel;
    if (sel >= 0 && sel < 6) this.cellDivs[sel].classList.add("sel");
  }

  /** Open the chat composer; the caller drops pointer lock. */
  openChat() {
    this.chatOpen = true;
    this.chat.style.display = "flex";
    this.chatlog.classList.add("held");
    this.chatinput.value = "";
    this.chatinput.focus();
  }

  closeChat() {
    this.chatOpen = false;
    this.chat.style.display = "none";
    this.chatlog.classList.remove("held");
    this.chatinput.blur();
  }

  /** One received line. `own` marks your own echo, which is the receipt
   * that the server actually relayed it — the client never renders a
   * line on faith. Names don't exist yet, so the speaker is their id. */
  chatLine(from, global, text, own) {
    while (this.chatlog.childElementCount >= CHAT_CAP) {
      this.chatlog.removeChild(this.chatlog.firstChild);
    }
    const div = document.createElement("div");
    div.className = `chatline${global ? " global" : ""}${own ? " own" : ""}`;
    const who = document.createElement("span");
    who.className = "who";
    who.textContent = `${global ? "[g] " : ""}#${from}`;
    div.appendChild(who);
    // textContent, never innerHTML: the line is another player's bytes.
    div.appendChild(document.createTextNode(` ${text}`));
    this.chatlog.appendChild(div);
    setTimeout(() => {
      if (div.parentNode === this.chatlog) this.chatlog.removeChild(div);
    }, CHAT_FADE_MS);
  }

  /**
   * Raise the death screen. `cause` is a `world::DEATH_BY_*` code, `killer`
   * the id that did it, `weapon` its display name (null for the world's own
   * kills), `range` metres.
   *
   * No position anywhere in the sentence, and that is the rule rather than
   * an omission (ALPHA.md §1, "no map position"): a screen that told you
   * where you fell would hand the raider standing over you a pin to the
   * base they just cleared. Who, with what, from how far.
   */
  showDeath(cause, killer, weapon, range, own) {
    // 0 = another player's hand, 1 = the survival clock, 2 = the sea.
    let line;
    if (cause === 1) {
      line = "you ran out";
    } else if (cause === 2) {
      line = "the sea is salt";
    } else if (own) {
      line = "you did it to yourself";
    } else {
      const with_ = weapon ? ` with ${weapon}` : "";
      line = `#${killer} killed you${with_} from ${range.toFixed(1)} m`;
    }
    this.deathCause.textContent = line;
    this.deathNote.textContent = "";
    this.respawnBag.disabled = false;
    this.respawnBeach.disabled = false;
    this.death.style.display = "flex";
    this.deathOpen = true;
  }

  /** The answer, once. The buttons disable on the click rather than on the
   * wake, so a second press cannot send a second action into a screen the
   * server has already closed — the sim ignores it (world.rs), and the
   * player should not be left wondering whether the first one took. */
  answerDeath(onBag) {
    if (!this.deathOpen || this.respawnBag.disabled) return;
    this.respawnBag.disabled = true;
    this.respawnBeach.disabled = true;
    this.deathNote.textContent = "waking…";
    this.onRespawn(onBag);
  }

  /** The wake landed. `onBag` is which anchor actually answered — asking
   * for a bag inside its cooldown gets a beach, and a player who is not
   * told that has no way to learn it except by looking around. */
  hideDeath(onBag, askedForBag) {
    this.death.style.display = "none";
    this.deathOpen = false;
    if (askedForBag && !onBag) {
      this.toast("no bag ready — you woke on a beach");
    } else if (onBag) {
      this.toast("you woke on your bag");
    }
  }

  /** One floating "+N Thing" line; oldest evicted past the cap. */
  toast(text) {
    while (this.toasts.childElementCount >= TOAST_CAP) {
      this.toasts.removeChild(this.toasts.firstChild);
    }
    const div = document.createElement("div");
    div.className = "toast";
    div.textContent = text;
    this.toasts.appendChild(div);
    setTimeout(() => {
      if (div.parentNode === this.toasts) this.toasts.removeChild(div);
    }, TOAST_MS);
  }
}
