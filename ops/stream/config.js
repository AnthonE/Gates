// Everything you'd want to change between streams lives here.
// Edit, save, then right-click the source in OBS -> Refresh (or toggle the scene).
window.GATES = {
  // Shown on the Starting Soon card and as the HUD's title line.
  today: "Fresh wipe — naked on the beach, raiding by sundown",

  // Starting Soon countdown, in minutes. Override per-source with ?min=10
  // or pin it to a wall-clock time with ?until=19:30 (24h, local time).
  countdownMinutes: 5,

  playUrl: "elopros.com/games/gates",
  site: "elopros.com",
  socials: [
    ["X", "@ELOPROS1337"],
    ["Discord", "discord.gg/UYg2D3Yudm"],
    ["Telegram", "t.me/elopros4life"],
  ],

  // Scrolling ticker lines. Keep them short; they loop forever.
  ticker: [
    "PLAY FREE → elopros.com/games/gates — browser or desktop, no install",
    "100-player shards · wake with nothing · gather · build · raid · lose it all · again",
    "JUNK / ELO pool is live on Abyss → app.abyss.trading",
    "Discord → discord.gg/UYg2D3Yudm",
    "X → @ELOPROS1337  ·  Telegram → t.me/elopros4life",
  ],

  // Live prices from DexScreener (free, no key). Set to [] to hide.
  tokens: [
    { symbol: "ELO",  address: "0xF7997e4fd821e39D884D86d25e1655017f241A42" },
    { symbol: "JUNK", address: "0xd6bd13a8a16e5249fa54961fc92eee0307e0f1b9" },
  ],

  // Background slideshow on the full-screen cards.
  art: [
    "art/gates-card.png",
    "art/gates-beast-1.jpg",
    "art/gates-play-1.jpg",
    "art/gates-play-5.jpg",
    "art/gates-play-8.jpg",
    "art/gates-play-11.jpg",
  ],
};
