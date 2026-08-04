// DOM HUD, updated on a slow timer well outside the RAF path (L8: UI in
// plain DOM outside the loop). Toasts are event-driven — they fire from
// the event-lane pump, which is also not the RAF path.

// The container kinds, taken from the module that already restates them from
// `sim-core/src/inventory.rs` under a gate, rather than restated a third time
// here. Every address this panel forms names its container explicitly — see
// the note on `invContainers` in the constructor.
import { CONT_SELF } from "./invmove.js";

/** Toast lifetime and cap (cosmetic; the stack reads like the reference
 * gather feedback — stacking "+N Thing" lines that fade). */
const TOAST_MS = 1600;
const TOAST_CAP = 8;

/** Chat log: lines kept on screen, and how long a line stays visible once
 * the composer is closed (cosmetic — the reference's kill/contacts feed
 * fades the same way). Open the composer and the whole log holds. */
const CHAT_CAP = 8;
const CHAT_FADE_MS = 12000;

/** The inventory screen's shape, mirroring what the sim already owns:
 * `INV_SLOTS = 30` (crates/sim-core/src/limits.rs) read as `30 * 2` u16
 * words at web/src/wasm.js:76, split by ALPHA.md §1 into "6 hotbar slots,
 * 24 inventory". These are not new numbers — they are the shipped ones,
 * named here so the panel's layout and the wire's slot indices cannot
 * drift apart silently. */
const INV_BELT = 6;
const INV_GRID = 24;
const INV_SLOTS = 30;

/**
 * What a refused move tells the player, indexed by `REFUSE_M_*`
 * (crates/sim-core/src/inventory.rs). Index 0 is "landed" and is never
 * shown. The sim keeps these as separate reasons on purpose and so does
 * this table: "it is gone" (the box was destroyed under you) and "it is out
 * of reach" (you walked away from it) are the same refusal to a byte-level
 * gate and completely different news to a player standing there.
 *
 * A reason the sim grows and this table has not is caught by ui_smoke,
 * which walks 1..REFUSE_M_MAX and requires a distinct non-empty string for
 * each — a silent `undefined` here would tell the player nothing at the one
 * moment the panel owes them a sentence.
 */
const MOVE_REFUSALS = [
  "",
  "that slot is not there",
  "there is nothing there",
  "you do not have that many",
  "there is no room for it",
  "it is gone",
  "it is out of reach",
  "those do not stack",
];

export class Hud {
  /**
   * The `onInvMove` a panel has while no host has claimed the move verb.
   *
   * Identity, not behaviour: a host arms the drag by assigning over this,
   * and `beginInvDrag` compares against it. Static rather than a private
   * module const so `ci/ui_smoke.mjs` can put the unarmed state back and
   * assert the panel offers no gesture it cannot perform — an arming rule
   * with no gate would be the comment-with-no-gate class again.
   */
  static NO_MOVE_HOST = () => false;

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
    // The inventory screen. Built once at construction like the hotbar and
    // the death screen — a panel that allocated 30 cells the frame a
    // player opened it would do it mid-step.
    //
    // It shows, it selects, and since the sim grew a refusal path (wire
    // v17) it drags: `beginInvDrag` / `dropInvDrag` / `invMoveVerdict`
    // below are the move verb, and the ordering law they enforce is the
    // one CLAUDE.md's item-move trap is about.
    //
    // WIRED TO THE SIM — main.js claims the verb, both directions. The
    // outbound half encodes with `client_action_move`; the inbound half
    // reads `APPLIED2_MOVE` off `client_applied2()` and unpacks the
    // verdict with `web/src/invmove.js`. That flag lives in a SECOND
    // applied word because word 0's bit 31 is `STREAM_ERR`: the verdict
    // shared that bit until the systems lane split it, and while it did,
    // a landed move and "undecodable bytes" arrived as the same word.
    // `invmove.js` has the history and what the split closed.
    //
    // A host claims the verb by assigning over `NO_MOVE_HOST`, and until
    // one does the gesture does not START. An affordance
    // that always refuses is worse than one that is absent: every drop
    // would dim a cell and toast "that will not move", which teaches the
    // player the panel is broken rather than that the verb is unbuilt.
    // `beginInvDrag` therefore refuses outright until a host claims the
    // verb, and a host claims it by assigning over `NO_MOVE_HOST` — there
    // is nothing separate to remember, so nothing separate to forget.
    this.inv = document.getElementById("inv");
    this.invGrid = document.getElementById("invgrid");
    this.invBelt = document.getElementById("invbelt");
    this.invDetail = document.getElementById("invdetail");
    this.invOpen = false;
    this.invSelected = -1;
    this.invFocus = -1;
    /** Set by main.js: (slot 0..INV_BELT-1) → void. Belt cells only. */
    this.onInvSelect = () => {};
    /**
     * Set by main.js: (fromKind, from, toKind, to) → did the frame go out?
     *
     * Both ends are ADDRESSES — a container kind and a slot within it —
     * because `Command::Move` has always taken two of them and this side
     * was passing `CONT_SELF` twice as a literal at the one call site. A
     * slot number alone is not an address: bag slot 3 and self slot 3 are
     * the same integer and different items, which is the aliasing that
     * makes the second panel gap 1 asks for unbuildable.
     *
     * The host owns the count, not this panel: `setInventory` is handed
     * strings and a string is not a stack size, so a panel that parsed
     * "wood ×8" back into an 8 would be inventing the payload it sends.
     * main.js reads the count off the same authoritative array it drew
     * from — the quantize-both-sides law, applied to containers.
     *
     * Returning false means the host would not carry that shape — either
     * the wire refused it (`client_action_move` → 0) or the host cannot
     * address that container yet — and nothing is drawn.
     */
    this.onInvMove = Hud.NO_MOVE_HOST;
    /**
     * The containers this panel DRAWS, and therefore the only ones whose
     * cells it may mutate, predict on, or roll back.
     *
     * One entry today. The judge's gap 1 ("there is nowhere to put
     * anything") adds `CONT_BAG` here together with a contents source, and
     * that is the whole of what this list is for: an address naming a
     * container that is not in it is refused rather than guessed at, so
     * the panel can never draw a prediction over a cell it does not own.
     */
    this.invContainers = [CONT_SELF];
    /** The slot being dragged, or -1. */
    this.invDrag = -1;
    /**
     * The container the dragged slot lives in, or -1 when nothing is held.
     *
     * Kept beside `invDrag` rather than folded into it because a slot
     * number is what every cell, every text array and every wire field is
     * keyed by; the kind is the second half of the address, not a
     * replacement for the first.
     */
    this.invDragKind = -1;
    /**
     * The `pointerId` that began the live drag, or null.
     *
     * One item is picked up by one pointer, so only that pointer may put it
     * down or call it off. Without this a second finger's release reaches
     * `dropInvDrag` holding the FIRST finger's source and sends a move
     * nobody gestured — the one-drag guard refuses the second pointer's
     * press but has never had anything to say about its release.
     */
    this.invDragPointer = null;
    /** The one move in flight, or null. See `dropInvDrag`. */
    this.invPending = null;
    this.invCells = [];
    this.invDivs = [];
    this.invTexts = [];
    for (let s = 0; s < INV_SLOTS; s++) {
      const cell = document.createElement("div");
      cell.className = "invcell";
      const label = document.createElement("span");
      cell.appendChild(label);
      (s < INV_BELT ? this.invBelt : this.invGrid).appendChild(cell);
      cell.addEventListener("click", () => this.focusSlot(s));
      // The drag, as real pointer events. `pointerdown` picks up and
      // `pointerup` drops, so the press and the release can land on
      // different cells — which is the whole gesture. A release outside any
      // cell is handled on the panel below, not here.
      // These thirty cells ARE the self container, so they say so rather
      // than leaning on the argument default: the cell that names its own
      // container is the one thing a second panel's cells cannot get wrong
      // by copying.
      cell.addEventListener("pointerdown", (e) => {
        if (e.button === 0) this.beginInvDrag(s, e.pointerId, CONT_SELF);
      });
      cell.addEventListener("pointerup", (e) => this.dropInvDrag(s, e.pointerId, CONT_SELF));
      this.invCells.push(label);
      this.invDivs.push(cell);
      this.invTexts.push("");
    }
    // A drag ends wherever the pointer is released, and in real play that is
    // usually NOT over the panel: press a cell, walk the cursor onto the
    // world, let go. Bound on `window` for exactly that case. With the
    // listener on `#inv` — where it was — that release was never seen at
    // all: `invDrag` kept pointing at the source, the cell kept its mark,
    // and the player's NEXT press was refused by the one-drag guard while
    // that press's release ran the drop against the stale source. Press
    // cell 8, and the sim is asked to move cell 3.
    //
    // Which is the item-move verb failing as an unasked-for mutation on a
    // container, i.e. the exact shape CLAUDE.md's trap list says the
    // reference shipped three times in 28 minutes. It is not a cosmetic
    // stuck highlight.
    //
    // Cell handlers still run first — they are deeper in the same bubble
    // path — so a real drop has already cleared the drag by the time this
    // sees the event and `cancelInvDrag` no-ops. Scoped to the pointer that
    // began the drag: another finger releasing elsewhere is not this drag
    // ending. `blur` is not scoped and cannot be — once the page loses
    // focus the release will never arrive at all, so a drag that survived
    // a blur survives forever.
    this.onWinPointerUp = (e) => {
      if (e.pointerId === this.invDragPointer) this.cancelInvDrag();
    };
    this.onWinBlur = () => this.cancelInvDrag();
    window.addEventListener("pointerup", this.onWinPointerUp);
    window.addEventListener("pointercancel", this.onWinPointerUp);
    window.addEventListener("blur", this.onWinBlur);
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

  /** Toggle the inventory screen; returns whether it is now open. */
  toggleInv() {
    this.invOpen = !this.invOpen;
    this.inv.style.display = this.invOpen ? "flex" : "none";
    // A drag cannot survive the panel it was started in. Closing mid-drag
    // and reopening would otherwise leave a cell marked and the next click
    // would drop into it.
    this.cancelInvDrag();
    return this.invOpen;
  }

  /**
   * Does an open panel own this key right now?
   *
   * main.js asks once, ahead of every verb, so the ordering law is one
   * function with a gate on it rather than a guard repeated per verb —
   * the shape the composer's `stopPropagation` already has, given a name.
   * A player reading their bag is not asking to eat, drink, place a wall
   * or unlock a door, and each of those spends something.
   *
   * The panel's own toggle and close keys are excluded, so the answer is
   * self-contained: main.js handles those two first, but nothing here
   * depends on it doing so.
   */
  eatsKey(code) {
    if (!this.invOpen) return false;
    return code !== "Tab" && code !== "Escape";
  }

  /**
   * Draw the 30 slots. `texts` is SLOT-INDEXED — `texts[s]` is slot `s`
   * as the sim numbers it, which is the whole of this function's contract
   * and the one thing worth gating about it.
   *
   * Slots 0..INV_BELT-1 are the belt row (the same six the hotbar draws,
   * the same six the digit keys select); INV_BELT..INV_SLOTS-1 are the
   * grid, in reading order. CLAUDE.md's positional-payload trap is
   * exactly this shape — the right value in the wrong position, invisible
   * to every byte-level check because every field has the same type — so
   * `ci/ui_smoke.mjs` group I writes a distinct string into each of the
   * 30 and reads back which cell holds it.
   *
   * Slow-timer only, and only a changed cell touches the DOM.
   */
  setInventory(texts) {
    const p = this.invPending;
    for (let s = 0; s < INV_SLOTS; s++) {
      const t = texts[s] || "";
      if (t === this.invTexts[s]) continue;
      // The server has restated a slot this panel is predicting on. Its
      // word is newer than the rollback snapshot, so a refusal that arrives
      // after this must NOT put the snapshot back — see `invMoveVerdict`.
      // Marked before the write, because the write is what makes the
      // snapshot stale.
      //
      // `texts` is the SELF container (this is the own-inventory diff), so
      // only an end of the pending move that is itself in self can be the
      // slot being restated. Without the kind, a bag-slot-3 to self-slot-7
      // move would count a write to self slot 3 as its own restatement and
      // give up a rollback it still owed.
      if (
        p &&
        ((p.fromKind === CONT_SELF && s === p.from) ||
          (p.toKind === CONT_SELF && s === p.to))
      )
        p.restated = true;
      this.setInvText(s, t);
    }
  }

  /**
   * Highlight the selected belt slot inside the panel. The belt row IS
   * the hotbar, so this takes the same `input.sel` `setSelected` does and
   * the two can never disagree about which slot is live.
   */
  setInvSelected(sel) {
    if (sel === this.invSelected) return;
    if (this.invSelected >= 0)
      this.invDivs[this.invSelected].classList.remove("sel");
    this.invSelected = sel;
    if (sel >= 0 && sel < INV_BELT) this.invDivs[sel].classList.add("sel");
  }

  /**
   * A click on a cell: name what is in it, and — on the belt only — make
   * it the live slot, which is the one verb this screen has and the one
   * the client already owned (`input.sel`, keys 1–6). Nothing here moves
   * an item: see the constructor's note on why the drag waits for the
   * sim's refusal path.
   */
  focusSlot(s) {
    if (this.invFocus >= 0) this.invDivs[this.invFocus].classList.remove("focus");
    this.invFocus = s;
    this.invDivs[s].classList.add("focus");
    this.drawInvDetail();
    if (s < INV_BELT) this.onInvSelect(s);
  }

  /**
   * Begin a drag. Returns whether one started.
   *
   * An empty slot starts nothing — there is no such thing as dragging
   * nothing somewhere, and letting it start would make every later step
   * reason about a move with no item in it.
   */
  beginInvDrag(s, pointerId = null, kind = CONT_SELF) {
    if (this.onInvMove === Hud.NO_MOVE_HOST) return false;
    if (this.invDrag >= 0) return false;
    // A container this panel does not draw has no cell to pick up from and
    // no text array to read, so the address is refused before anything is
    // marked. Checked here and not only at the drop, because a drag that
    // starts somewhere unownable has already put a mark on the screen.
    if (!this.drawsContainer(kind)) return false;
    if (!this.invTexts[s]) return false;
    this.invDrag = s;
    this.invDragKind = kind;
    this.invDragPointer = pointerId;
    this.invDivs[s].classList.add("drag");
    return true;
  }

  /** Does this panel draw container `kind`? See `invContainers`. */
  drawsContainer(kind) {
    return this.invContainers.includes(kind);
  }

  /**
   * Abort a drag without moving anything. Four callers and all four are
   * wired: `pointerup` and `pointercancel` away from a cell and `blur`, all
   * three on `window` from the constructor, and Escape via `toggleInv`.
   *
   * Unconditional by design — the pointer-scoping lives at the window
   * listener, because `blur` and Escape have no pointer to scope to.
   */
  cancelInvDrag() {
    if (this.invDrag < 0) return false;
    this.invDivs[this.invDrag].classList.remove("drag");
    this.invDrag = -1;
    this.invDragKind = -1;
    this.invDragPointer = null;
    return true;
  }

  /**
   * Drop the drag on slot `to`. This is the item-move verb, and CLAUDE.md's
   * trap list is explicit about how it fails: three Oxide fixes in 28
   * minutes on one 2019 day, all one-line splice-point moves, all landing
   * as *the server disconnecting the client*, because container state
   * diverged and a diverged container reads as a forged request. The bug is
   * validation ORDERING against the mutation, never arithmetic.
   *
   * So the order here is fixed, and every step of it is gated:
   *
   *   1. validate the address BEFORE touching a cell — no drag, a release
   *      from a pointer that never picked this up, same slot, out of range,
   *      empty source. A refused drag mutates nothing, so there is nothing
   *      to roll back and no frame goes out. The pointer check in
   *      particular has to sit above `cancelInvDrag` and not below it, or a
   *      foreign release calls off a drag that is still under a finger —
   *      the validation-ordering law applied to the abort as well as to
   *      the move.
   *   2. snapshot exactly the two labels about to change.
   *   3. ask the host to encode and send. `client_action_move` returns 0
   *      for a shape the wire will not carry, and a drawn move with no
   *      frame behind it is the divergence itself — so the send is asked
   *      BEFORE the prediction is drawn, and a refusal to encode draws
   *      nothing at all.
   *   4. only then draw it.
   *
   * One move in flight at a time. The second concurrent splice is what the
   * reference actually shipped three times, and serialising is cheaper than
   * reconciling two predictions against one authoritative diff.
   */
  dropInvDrag(to, pointerId = null, toKind = CONT_SELF) {
    const from = this.invDrag;
    const fromKind = this.invDragKind;
    if (from < 0) return false;
    // Only the pointer that picked the item up may put it down. A second
    // finger's release arriving here holds the FIRST finger's `from`, and
    // honouring it sends a move the player never gestured — the same
    // unasked-for mutation the window-level cancel closes, by the other
    // door. Above the cancel, so a foreign release neither moves nor aborts.
    if (pointerId !== this.invDragPointer) return false;
    this.cancelInvDrag();
    if (this.invPending) {
      this.toast("still moving that");
      return false;
    }
    // Same ADDRESS, not same slot number. Self slot 3 to bag slot 3 is a
    // real move and the sim answers it; refusing it as a no-op because the
    // integers match is the aliasing this whole change exists to remove.
    if (to === from && toKind === fromKind) return false;
    if (!(to >= 0 && to < INV_SLOTS)) return false;
    // The destination has to be a container this panel draws, for the same
    // reason the source does: the prediction below writes two cells, and a
    // cell in a container the panel does not own does not exist to write.
    if (!this.drawsContainer(toKind)) return false;
    if (!this.invTexts[from]) return false;
    const wasFrom = this.invTexts[from];
    const wasTo = this.invTexts[to];
    if (!this.onInvMove(fromKind, from, toKind, to)) {
      // The host would not carry that shape — the wire refused it, or the
      // host cannot address that container yet. Nothing was drawn, so
      // nothing unwinds, and the player still has to learn the drag did
      // not happen.
      this.toast("that will not move");
      return false;
    }
    this.invPending = { fromKind, from, toKind, to, wasFrom, wasTo, restated: false };
    this.setInvText(from, "");
    this.setInvText(to, wasFrom);
    return true;
  }

  /**
   * The sim's verdict on the move this panel drew.
   *
   * `reason` is 0 for landed, else an `inventory.rs` `REFUSE_M_*`. The
   * address comes back with it because the refusal must be matched against
   * the prediction it answers: a verdict carrying a different `from`/`to`
   * than the one in flight is not this panel's move, and rolling the drawn
   * move back on it would corrupt a slot the server never spoke about.
   * That is the same positional-payload shape CLAUDE.md names — the right
   * value in the wrong position — one level up from the encoder.
   *
   * Both kinds are matched too, and they default to `CONT_SELF` because
   * that is the only container `invmove.moveVerdict` will hand up: it
   * rejects a readout whose FROM kind is not self, and the readout word
   * has no room to state the TO kind at all (`bridge.rs`'s
   * `client_move_readout`). So a caller that omits them is saying "a self
   * to self verdict", which is the only thing the wire can currently
   * deliver — while a pending move addressed anywhere else is left
   * unresolved rather than answered by a verdict about somewhere else.
   * Widening that word is the systems request on `NOW.md`.
   */
  invMoveVerdict(reason, from, to, fromKind = CONT_SELF, toKind = CONT_SELF) {
    const p = this.invPending;
    if (!p) return false;
    if (from !== p.from || to !== p.to) return false;
    if (fromKind !== p.fromKind || toKind !== p.toKind) return false;
    this.invPending = null;
    if (reason === 0) return true;
    // Roll back to what was drawn over — UNLESS an authoritative
    // `setInventory` has already restated those slots while this was in
    // flight. The server's word is newer than our snapshot, and restoring
    // the snapshot over it would put back an item the sim has since moved
    // somewhere else. This flag is the whole reason `setInventory` knows
    // about pending moves at all.
    // Only cells this panel actually drew are put back. A move with one end
    // in a container the panel does not draw drew ONE cell, and restoring
    // the other would write a slot number into the self grid that named a
    // different container's item — the aliasing, arriving by the back door.
    if (!p.restated) {
      if (this.drawsContainer(p.fromKind)) this.setInvText(p.from, p.wasFrom);
      if (this.drawsContainer(p.toKind)) this.setInvText(p.to, p.wasTo);
    }
    this.toast(MOVE_REFUSALS[reason] || "that will not move");
    return true;
  }

  /** One cell's label, kept with the mirror `setInventory` diffs against. */
  setInvText(s, t) {
    this.invTexts[s] = t;
    this.invCells[s].textContent = t;
    if (s === this.invFocus) this.drawInvDetail();
  }

  /** The readout under the grid. Belt slots are named by their digit key
   * (1–6); grid slots are numbered 1–24 within the grid, because there is
   * no key 7 and a readout that implied one would be lying. */
  drawInvDetail() {
    const s = this.invFocus;
    if (s < 0) {
      this.invDetail.textContent = "";
      return;
    }
    const where = s < INV_BELT ? `belt ${s + 1}` : `slot ${s - INV_BELT + 1}`;
    this.invDetail.textContent = `${where} · ${this.invTexts[s] || "empty"}`;
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
    // The bag you were reading is not yours any more — it is lying on the
    // ground with your body in it. Close the screen rather than leave it
    // showing a corpse's slots behind the respawn buttons.
    if (this.invOpen) this.toggleInv();
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
