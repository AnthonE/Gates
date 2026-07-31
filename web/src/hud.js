// DOM HUD, updated on a slow timer well outside the RAF path (L8: UI in
// plain DOM outside the loop). Toasts are event-driven — they fire from
// the event-lane pump, which is also not the RAF path.

/** Toast lifetime and cap (cosmetic; the stack reads like the reference
 * gather feedback — stacking "+N Thing" lines that fade). */
const TOAST_MS = 1600;
const TOAST_CAP = 8;

export class Hud {
  constructor() {
    this.el = document.getElementById("hud");
    this.cross = document.getElementById("cross");
    this.hotbar = document.getElementById("hotbar");
    this.toasts = document.getElementById("toasts");
    this.craft = document.getElementById("craft");
    this.craftq = document.getElementById("craftq");
    this.craftOpen = false;
    this.last = "";
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
