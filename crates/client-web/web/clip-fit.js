/* Keep the film's canvas whole inside the frame. The game sizes the canvas to
   the video (its CSS box is the shot list's pixels), so the box around it is
   scaled instead, every frame the frame or the canvas changes. */
(() => {
  const stage = document.getElementById("stage");
  const film = document.getElementById("film");
  let last = "";
  function fit() {
    const w = film.offsetWidth || film.width, h = film.offsetHeight || film.height;
    const s = Math.min(innerWidth / w, innerHeight / h, 1);
    const key = `${w}x${h}@${innerWidth}x${innerHeight}`;
    if (key !== last) {
      last = key;
      stage.style.transform = `scale(${s}) translate(-50%, -50%)`;
    }
    requestAnimationFrame(fit);
  }
  requestAnimationFrame(fit);
})();
