// Shared behaviour: the mark, the ticker (with live DexScreener prices), and the
// drifting spores from the irradiated zones. Every overlay loads config.js first.
(function () {
  const C = window.GATES;

  const MARK = '<svg viewBox="0 0 64 64"><rect x="1" y="1" width="62" height="62" rx="14" fill="#b8ff38"/>' +
    '<path fill="#0a0c0a" d="M14 26 L32 11 L50 26 L50 35 L32 20 L14 35 Z M14 44 L32 29 L50 44 L50 53 L32 38 L14 53 Z"/></svg>';
  document.querySelectorAll(".mark").forEach((el) => (el.innerHTML = MARK));

  const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

  // ---- ticker ----
  window.mountTicker = function (host) {
    host.classList.add("ticker");
    host.innerHTML =
      '<div class="prices"></div><div class="lane"><div class="belt"></div></div>' +
      '<div class="end">▶ ' + esc(C.playUrl) + "</div>";
    if (!C.tokens.length) host.querySelector(".prices").remove();

    // Two identical copies of the message run, so the loop seam is invisible.
    const belt = host.querySelector(".belt");
    const run = C.ticker.map((t) => "<span>" + esc(t) + "</span><em>◆</em>").join("");
    belt.innerHTML = run + run;
    let x = 0, last = performance.now();
    (function step(now) {
      x -= ((now - last) / 1000) * 70; // px per second
      last = now;
      const half = belt.scrollWidth / 2;
      if (half && -x >= half) x += half;
      belt.style.transform = "translateX(" + x + "px)";
      requestAnimationFrame(step);
    })(last);

    if (C.tokens.length) refreshPrices(host.querySelector(".prices"));
  };

  // Small-cap prices need significant digits, not cents.
  function fmt(n) {
    if (n == null || isNaN(n)) return "—";
    if (n >= 1) return n.toLocaleString("en-US", { maximumFractionDigits: 4 });
    const d = Math.max(2, -Math.floor(Math.log10(n)) + 2);
    return n.toFixed(Math.min(d, 10));
  }
  function chg(p) {
    if (p == null) return "";
    const up = p >= 0;
    return '<span class="chg ' + (up ? "up" : "down") + '">' + (up ? "▲" : "▼") + Math.abs(p).toFixed(1) + "%</span>";
  }

  async function refreshPrices(box) {
    const rows = [];
    let eloUsd = null;
    for (const t of C.tokens) {
      try {
        const r = await fetch("https://api.dexscreener.com/latest/dex/tokens/" + t.address);
        const pairs = ((await r.json()).pairs || []).filter(
          (p) => p.baseToken.address.toLowerCase() === t.address.toLowerCase()
        );
        // Most liquid pair where this token is the base.
        pairs.sort((a, b) => ((b.liquidity || {}).usd || 0) - ((a.liquidity || {}).usd || 0));
        const p = pairs[0];
        if (!p) continue;
        const ch = (p.priceChange || {}).h24;
        if (p.priceUsd) {
          const usd = parseFloat(p.priceUsd);
          if (t.symbol === "ELO") eloUsd = usd;
          rows.push('<div class="px"><b>' + esc(t.symbol) + "</b>$" + fmt(usd) + chg(ch) + "</div>");
        } else {
          // Quoted in another token (JUNK trades against ELO): show both.
          const nat = parseFloat(p.priceNative);
          const q = p.quoteToken.symbol;
          const usd = q === "ELO" && eloUsd ? " ≈$" + fmt(nat * eloUsd) : "";
          rows.push('<div class="px"><b>' + esc(t.symbol) + "</b>" + fmt(nat) + " " + esc(q) +
            '<span class="sub">' + usd + "</span>" + chg(ch) + "</div>");
        }
      } catch (e) { /* offline or rate-limited: keep the last good numbers */ }
    }
    if (rows.length) box.innerHTML = rows.join("");
    setTimeout(() => refreshPrices(box), 60000);
  }

  // ---- spores: lime motes drifting up, like the glow-zones in-game ----
  window.mountSpores = function (canvas, count) {
    const ctx = canvas.getContext("2d");
    const W = (canvas.width = canvas.offsetWidth), H = (canvas.height = canvas.offsetHeight);
    const motes = Array.from({ length: count }, () => spawn(true));
    function spawn(anywhere) {
      return {
        x: Math.random() * W, y: anywhere ? Math.random() * H : H + 10,
        vx: (Math.random() - 0.5) * 12, vy: -(8 + Math.random() * 26),
        len: 4 + Math.random() * 14, a: 0.25 + Math.random() * 0.6, ph: Math.random() * 6.28,
      };
    }
    let last = performance.now();
    (function frame(now) {
      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      ctx.clearRect(0, 0, W, H);
      ctx.lineCap = "round";
      for (let i = 0; i < motes.length; i++) {
        const m = motes[i];
        m.ph += dt * 1.3;
        m.x += (m.vx + Math.sin(m.ph) * 10) * dt;
        m.y += m.vy * dt;
        if (m.y < -20) motes[i] = spawn(false);
        const flick = 0.55 + 0.45 * Math.sin(m.ph * 3);
        ctx.strokeStyle = "rgba(184,255,56," + (m.a * flick).toFixed(3) + ")";
        ctx.shadowColor = "#b8ff38";
        ctx.shadowBlur = 8;
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.moveTo(m.x, m.y);
        ctx.lineTo(m.x - m.vx * 0.15, m.y + m.len);
        ctx.stroke();
      }
      requestAnimationFrame(frame);
    })(last);
  };
})();
