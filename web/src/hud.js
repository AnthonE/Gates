// DOM HUD, updated on a slow timer well outside the RAF path (L8: UI in
// plain DOM outside the loop).

export class Hud {
  constructor() {
    this.el = document.getElementById("hud");
    this.cross = document.getElementById("cross");
    this.last = "";
  }

  show() {
    this.el.style.display = "block";
    this.cross.style.display = "block";
  }

  set(text) {
    if (text !== this.last) {
      this.last = text;
      this.el.textContent = text;
    }
  }
}
