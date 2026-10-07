#!/usr/bin/env python3
"""Cut an ad from film shots: `ci/film_cut.py CUT.json [more.json ...]`.

The shots come from `gates --film` (`render/film.rs`): each is `NN-name.mp4`
with the game's own mix beside it as `NN-name.wav`. A cut list names clips
from them, captions, music and sound effects from the game's own bank
(`cargo run -p client --bin soundbank -- DIR`), and an end card, and this
renders the ad with ffmpeg in one pass. Captions and the card are drawn with
PIL in the game's own face (Roboto Condensed), since ads mostly play muted.

    {
      "out": "film/ads/trailer.mp4", "size": [1920, 1080], "fps": 30,
      "shots": "film/shots/wide",            # dir (or dirs) of NN-name.mp4 + .wav
      "bank": "film/bank",                   # soundbank dir (music, sfx)
      "fade": 0.3,                           # default crossfade, seconds
      "clips": [{"shot": "beach", "in": 0.5, "len": 3.5,
                 "game": 0.8, "fade": 0.5}],
      "captions": [{"at": 0.6, "len": 2.4, "text": "WAKE WITH NOTHING",
                    "size": 0.07, "y": 0.78}],
      "music": [{"src": "29_MusicOpenCalm.wav", "at": 0, "gain": 0.9}],
      "sfx": [{"src": "54_Blast_0.wav", "at": 12.3, "gain": 1.0}],
      "card": {"len": 4, "shot": "beach", "at": 2.0,
               "title": "GATES", "lines": ["...", "..."]}
    }

More, all optional:

    "grade": "curves=...",      # every clip's ffmpeg grade unless it has one
    clip "transition": "fade" | "black" | "white" | "cut"   # into this clip
    clip "speed": 0.5            # retime the clip (slow it down, speed it up)
    "finish": {"halation": 0.15, # highlights bloom into a warm glow
               "vignette": 0.4,  # radians, ffmpeg's vignette angle
               "grain": 5,       # temporal luma grain
               "letterbox": 2.39},  # bars to this aspect
    "flashes": [{"at": 14.1, "len": 0.2}],   # a white frame that fades
    music/sfx "in": 3.0, "fade_in": 0.5      # start partway in, ease in

`ci/film_sfx.py DIR` writes trailer hits, risers and whooshes into a bank.
"""

import json
import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BOLD = os.path.join(REPO, "crates/client/fonts/RobotoCondensed-Bold.ttf")
REGULAR = os.path.join(REPO, "crates/client/fonts/RobotoCondensed-Regular.ttf")


def run(cmd):
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        sys.stderr.write(r.stderr[-4000:])
        sys.exit(f"film_cut: ffmpeg failed ({r.returncode})")
    return r.stdout


def duration(path):
    r = subprocess.run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                        "-of", "csv=p=0", path], capture_output=True, text=True)
    try:
        return float(r.stdout.strip())
    except ValueError:
        return 0.0


def find_shot(shots_dirs, name, ext):
    """`shots_dirs` is one directory or several, searched in order."""
    for d in shots_dirs:
        for f in sorted(os.listdir(d)):
            stem, e = os.path.splitext(f)
            if e == ext and stem.split("-", 1)[-1] == name:
                return os.path.join(d, f)
    sys.exit(f"film_cut: no shot `{name}` ({ext}) in {', '.join(shots_dirs)}")


def tracked(draw, xy, text, font, fill, tracking):
    """Draw `text` with extra letter spacing; returns its width."""
    x, y = xy
    for ch in text:
        draw.text((x, y), ch, font=font, fill=fill)
        x += draw.textlength(ch, font=font) + tracking
    return x - xy[0] - tracking


def text_width(draw, text, font, tracking):
    return sum(draw.textlength(ch, font=font) for ch in text) + tracking * max(len(text) - 1, 0)


def text_layer(W, H, lines, shadow=True):
    """A transparent full-frame layer with centred lines of text.

    `lines` is a list of (text, font_path, size_px, y_centre_px, rgba,
    tracking_px). A line wider than 90% of the frame is shrunk to fit, so one
    cut list serves 16:9, 1:1 and 9:16.
    """
    img = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    glyphs = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    d = ImageDraw.Draw(glyphs)
    for text, face, size, yc, fill, tracking in lines:
        font = ImageFont.truetype(face, size)
        w = text_width(d, text, font, tracking)
        if w > W * 0.9:
            k = W * 0.9 / w
            size, tracking = max(8, int(size * k)), tracking * k
            font = ImageFont.truetype(face, size)
            w = text_width(d, text, font, tracking)
        asc, desc = font.getmetrics()
        tracked(d, ((W - w) / 2, yc - (asc + desc) / 2), text, font, fill, tracking)
    if shadow:
        alpha = glyphs.split()[3]
        sh = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        sh.putalpha(alpha.point(lambda a: int(a * 0.75)))
        sh = sh.filter(ImageFilter.GaussianBlur(max(2, H // 180)))
        img = Image.alpha_composite(img, sh)
    return Image.alpha_composite(img, glyphs)


def caption_png(path, W, H, cap):
    size = int(H * cap.get("size", 0.07) if W >= H else W * cap.get("size", 0.07) * 1.25)
    yc = int(H * cap.get("y", 0.8))
    lines = [(cap["text"], BOLD, size, yc, (255, 255, 255, 255), size * 0.06)]
    if cap.get("sub"):
        s2 = int(size * 0.5)
        lines.append((cap["sub"].upper(), REGULAR, s2, yc + int(size * 0.9), (240, 232, 218, 255), s2 * 0.1))
    text_layer(W, H, lines).save(path)


def card_bg(W, H, card, bg_frame):
    """The card's backdrop: a frame covering the canvas, softened and dimmed."""
    bg = Image.open(bg_frame).convert("RGB")
    # Cover, not stretch: a 16:9 frame behind a 9:16 card is cropped to fit.
    k = max(W / bg.width, H / bg.height)
    bg = bg.resize((round(bg.width * k), round(bg.height * k)), Image.LANCZOS)
    left, top = (bg.width - W) // 2, (bg.height - H) // 2
    bg = bg.crop((left, top, left + W, top + H))
    if card.get("blur", True):
        bg = bg.filter(ImageFilter.GaussianBlur(H // 120))
    shade = Image.new("RGB", (W, H), (8, 8, 10))
    return Image.blend(bg, shade, card.get("dim", 0.55))


def card_text(W, H, card):
    """The card's words: the title, then its lines, centred."""
    portrait = H > W
    unit = W if portrait else H
    title = int(unit * (0.22 if portrait else 0.20))
    lines = [(card["title"], BOLD, title, int(H * 0.42), (255, 255, 255, 255), title * 0.18)]
    y = int(H * 0.42) + int(title * 0.78)
    for i, line in enumerate(card.get("lines", [])):
        s = int(unit * (0.055 if portrait else 0.05)) if i == 0 else int(unit * (0.042 if portrait else 0.036))
        face = BOLD if i == 0 else REGULAR
        colour = (255, 255, 255, 255) if i == 0 else (255, 196, 120, 255)
        lines.append((line, face, s, y, colour, s * 0.1))
        y += int(s * 1.55)
    return text_layer(W, H, lines)


def card_png(path, W, H, card, bg_frame):
    """The card as one still, for a poster."""
    bg = card_bg(W, H, card, bg_frame).convert("RGBA")
    Image.alpha_composite(bg, card_text(W, H, card)).convert("RGB").save(path)


def cut(spec_path):
    spec = json.load(open(spec_path))

    def rel(p):
        # Relative to the working directory, like `gates --film`'s paths:
        # `ci/film.sh` runs both from the repo root.
        return os.path.abspath(p)

    W, H = spec["size"]
    fps = spec.get("fps", 30)
    shots = [rel(d) for d in (spec["shots"] if isinstance(spec["shots"], list) else [spec["shots"]])]
    bank = rel(spec.get("bank", "bank"))
    default_fade = spec.get("fade", 0.3)
    clips = spec["clips"]
    tmp = tempfile.mkdtemp(prefix="film_cut_")

    args = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"]
    vf, af = [], []
    n_in = 0

    def add_input(*a):
        nonlocal n_in
        args.extend(a)
        n_in += 1
        return n_in - 1

    # ---- picture: clips, crossfaded end to end
    t = 0.0
    starts = []
    prev = None
    for k, c in enumerate(clips):
        src = find_shot(shots, c["shot"], ".mp4")
        sp = c.get("speed", 1.0)
        have = duration(src) - c.get("in", 0.0)
        if have + 1e-3 < c["len"] * sp:
            print(f"film_cut: `{c['shot']}` has {have:.2f} s after `in`, the clip asks "
                  f"{c['len'] * sp:.2f} s — its last frame is held", file=sys.stderr)
        i = add_input("-ss", str(c.get("in", 0.0)), "-t", str(c["len"] * sp), "-i", src)
        chain = f"[{i}:v]"
        if sp != 1.0:
            # Retimed with motion-compensated tweening, so a slowed shot
            # gains frames rather than holding each one twice.
            chain += f"setpts=PTS/{sp},"
            if sp < 1.0:
                chain += f"minterpolate=fps={fps}:mi_mode=mci:mc_mode=aobmc:vsbmc=1,"
        chain += f"fps={fps},scale={W}:{H}:force_original_aspect_ratio=increase:flags=lanczos,crop={W}:{H},setsar=1"
        if c.get("push"):
            # A slow push-in: scale up over the clip and crop the centre.
            z = c["push"]
            chain += f",scale=w='iw*(1+{z}*t/{c['len']})':h=-2:eval=frame:flags=lanczos,crop={W}:{H}"
        if c.get("grade", spec.get("grade")):
            chain += "," + c.get("grade", spec.get("grade"))
        # Exactly `len` long whatever the source holds: every offset after
        # this clip is computed from it, and a short clip pulled the end
        # card's crossfade past the end of the picture.
        chain += f",tpad=stop_mode=clone:stop_duration={c['len']},trim=duration={c['len']}"
        vf.append(chain + f",format=yuv420p,setpts=PTS-STARTPTS,settb=1/{fps}[c{k}]")
        if prev is None:
            prev = f"c{k}"
            starts.append(0.0)
            t = c["len"]
        else:
            kind = c.get("transition", "fade")
            f = 0 if kind == "cut" else c.get("fade", default_fade)
            if f > 0:
                off = t - f
                how = {"fade": "fade", "black": "fadeblack", "white": "fadewhite"}[kind]
                vf.append(f"[{prev}][c{k}]xfade=transition={how}:duration={f}:offset={off:.4f}[x{k}]")
            else:
                off = t
                # `concat` hands back the microsecond timebase, which the
                # next `xfade` refuses to meet, so it is pinned back.
                vf.append(f"[{prev}][c{k}]concat=n=2:v=1:a=0,settb=1/{fps}[x{k}]")
            starts.append(off)
            prev = f"x{k}"
            t = off + c["len"]

    # ---- the end card, crossfaded in from the last clip
    card = spec.get("card")
    if card:
        frame = os.path.join(tmp, "card_bg.png")
        run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", str(card.get("at", 1.0)),
             "-i", find_shot(shots, card["shot"], ".mp4"), "-frames:v", "1", frame])
        bg_png, text_png = os.path.join(tmp, "card_bg.png"), os.path.join(tmp, "card_text.png")
        card_bg(W, H, card, frame).save(bg_png)
        card_text(W, H, card).save(text_png)
        loop = ("-loop", "1", "-framerate", str(fps), "-t", str(card["len"]))
        ib = add_input(*loop, "-i", bg_png)
        it = add_input(*loop, "-i", text_png)
        # The backdrop pushes in slowly under words that hold still, so a
        # long card is still a moving picture.
        z, ln = card.get("push", 0.06), card["len"]
        vf.append(f"[{ib}:v]scale=w='{W}*(1+{z}*t/{ln})':h=-2:eval=frame:flags=lanczos,"
                  f"crop={W}:{H},setsar=1[cardbg]")
        vf.append(f"[cardbg][{it}:v]overlay=0:0:format=auto,format=yuv420p,settb=1/{fps}[card]")
        f = card.get("fade", 0.6)
        off = t - f
        vf.append(f"[{prev}][card]xfade=transition=fade:duration={f}:offset={off:.4f}[xc]")
        prev = "xc"
        t = off + card["len"]
    total = t

    # ---- the finish: the lab's half of the look, over everything but words
    fin = spec.get("finish", {})
    if fin.get("halation"):
        # Only what is near white glows: the blurred copy is cut to the
        # highlights first, so the blacks stay black.
        sig = max(4, H // 54)
        vf.append(f"[{prev}]format=gbrp,split[h0][h1];"
                  f"[h1]curves=all='0/0 0.62/0 1/1',gblur=sigma={sig},"
                  f"colorchannelmixer=rr=1:gg=0.62:bb=0.38[h2];"
                  f"[h0][h2]blend=all_mode=screen:all_opacity={fin['halation']},format=yuv420p[hal]")
        prev = "hal"
    post = []
    if fin.get("vignette"):
        post.append(f"vignette=angle={fin['vignette']}")
    if fin.get("grain"):
        post.append(f"noise=c0s={fin['grain']}:c0f=t+u")
    if fin.get("letterbox"):
        bar = max(0, round((H - W / fin["letterbox"]) / 2))
        post.append(f"drawbox=x=0:y=0:w=iw:h={bar}:color=black:t=fill,"
                    f"drawbox=x=0:y=ih-{bar}:w=iw:h={bar}:color=black:t=fill")
    if post:
        vf.append(f"[{prev}]{','.join(post)}[fin]")
        prev = "fin"
    for k, fl in enumerate(spec.get("flashes", [])):
        a, ln = fl["at"], fl.get("len", 0.2)
        i = add_input("-f", "lavfi", "-i", f"color=c=white:s={W}x{H}:r={fps}:d={a + ln}")
        vf.append(f"[{i}:v]format=rgba,fade=t=out:st={a}:d={ln}:alpha=1[fl{k}]")
        vf.append(f"[{prev}][fl{k}]overlay=0:0:enable='between(t,{a},{a + ln})'[f{k}]")
        prev = f"f{k}"

    # ---- captions, each a still layer faded in and out over the picture
    for k, cap in enumerate(spec.get("captions", [])):
        png = os.path.join(tmp, f"cap{k}.png")
        caption_png(png, W, H, cap)
        a, ln = cap["at"], cap["len"]
        fi = min(0.25, ln / 4)
        i = add_input("-loop", "1", "-framerate", str(fps), "-t", str(a + ln), "-i", png)
        vf.append(f"[{i}:v]format=rgba,fade=t=in:st={a}:d={fi}:alpha=1,"
                  f"fade=t=out:st={a + ln - fi}:d={fi}:alpha=1[cap{k}]")
        vf.append(f"[{prev}][cap{k}]overlay=0:0:enable='between(t,{a},{a + ln})'[o{k}]")
        prev = f"o{k}"
    if spec.get("fade_in", 0.0) > 0 or spec.get("fade_out", 0.0) > 0:
        fx = []
        if spec.get("fade_in", 0.0) > 0:
            fx.append(f"fade=t=in:st=0:d={spec['fade_in']}")
        if spec.get("fade_out", 0.0) > 0:
            fx.append(f"fade=t=out:st={total - spec['fade_out']}:d={spec['fade_out']}")
        vf.append(f"[{prev}]{','.join(fx)}[vfaded]")
        prev = "vfaded"
    vf.append(f"[{prev}]format=yuv420p[vout]")

    # ---- sound: each clip's own game mix, then music and effects over it
    mix = []
    for k, c in enumerate(clips):
        g = c.get("game", 0.7)
        if g <= 0:
            continue
        src = find_shot(shots, c["shot"], ".wav")
        sp = c.get("speed", 1.0)
        i = add_input("-ss", str(c.get("in", 0.0)), "-t", str(c["len"] * sp), "-i", src)
        f = min(c.get("fade", default_fade), c["len"] / 3) or 0.05
        ms = int(starts[k] * 1000)
        # A slowed clip's sound drops in pitch with it, which is the sound
        # slow motion makes in every trailer.
        retime = f"asetrate={int(48000 * sp)},aresample=48000," if sp != 1.0 else ""
        af.append(f"[{i}:a]aresample=48000,{retime}volume={g},afade=t=in:d={f},"
                  f"afade=t=out:st={c['len'] - f}:d={f},adelay={ms}|{ms}[g{k}]")
        mix.append(f"[g{k}]")
    for k, m in enumerate(spec.get("music", []) + spec.get("sfx", [])):
        src = rel(os.path.join(bank, m["src"])) if not os.path.isabs(m["src"]) else m["src"]
        i = add_input("-ss", str(m.get("in", 0.0)), "-i", src)
        ms = int(m["at"] * 1000)
        chain = f"[{i}:a]aresample=48000,aformat=channel_layouts=stereo,volume={m.get('gain', 1.0)}"
        if m.get("fade_in"):
            chain += f",afade=t=in:d={m['fade_in']}"
        if m.get("cut"):
            chain += f",atrim=0:{m['cut']}"
        if m.get("fade_out"):
            end = m.get("cut", m.get("len", 10.5))
            chain += f",afade=t=out:st={end - m['fade_out']}:d={m['fade_out']}"
        af.append(chain + f",adelay={ms}|{ms}[m{k}]")
        mix.append(f"[m{k}]")
    if mix:
        # Padded to the picture's length: the card can outlast every sound.
        af.append(f"{''.join(mix)}amix=inputs={len(mix)}:normalize=0:duration=longest,"
                  f"apad=whole_dur={total:.4f},atrim=0:{total:.4f},afade=t=out:st={max(total - 1.0, 0):.4f}:d=1,"
                  f"loudnorm=I={spec.get('loudness', -14)}:TP=-1.5:LRA=11,aresample=48000[aout]")

    out = rel(spec["out"])
    os.makedirs(os.path.dirname(out), exist_ok=True)

    # A cover image for the platforms that want one (YouTube's thumbnail, a
    # store capsule): the card's design over a chosen frame, at full size.
    poster = spec.get("poster")
    if poster:
        frame = os.path.join(tmp, "poster_bg.png")
        if poster.get("image"):
            # A still from `gates --film` (`still: true`) rather than a video
            # frame: full resolution, converged, no motion blur.
            frame = rel(poster["image"])
        else:
            run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-ss", str(poster.get("at", 1.0)),
                 "-i", find_shot(shots, poster["shot"], ".mp4"), "-frames:v", "1", frame])
        design = dict(card or {}, **{k: v for k, v in poster.items() if k in ("title", "lines", "dim")})
        design.setdefault("dim", 0.3)
        design["blur"] = False
        PW, PH = poster.get("size", [W, H])
        card_png(rel(poster["out"]), PW, PH, design, frame)
        print(f"film_cut: {rel(poster['out'])}")
    args += ["-filter_complex", ";".join(vf + af), "-map", "[vout]"]
    if mix:
        args += ["-map", "[aout]", "-c:a", "aac", "-b:a", "192k", "-ar", "48000"]
    # Capped where the platforms' own upload guidance sits for 1080p30
    # (~8 Mbps), so an ad is small enough to send and loses nothing on upload.
    args += ["-t", f"{total:.4f}", "-r", str(fps), "-c:v", "libx264", "-preset", "slow",
             "-crf", str(spec.get("crf", 18)), "-maxrate", spec.get("maxrate", "7M"),
             "-bufsize", "14M", "-profile:v", "high", "-pix_fmt", "yuv420p",
             "-movflags", "+faststart", out]
    run(args)
    print(f"film_cut: {out} — {total:.2f} s, {W}x{H}")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    for p in sys.argv[1:]:
        cut(p)
