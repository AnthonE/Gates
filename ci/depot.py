#!/usr/bin/env python3
"""Package a desktop build as a scry depot.

    ./ci/depot.py                       build + stage + write the depot index
    ./ci/depot.py --platform win-x86_64 the Windows depot (x86_64-pc-windows-gnu)
    ./ci/depot.py --root https://...    bake a different download root
    ./ci/depot.py --no-build            package whatever is already compiled
    ./ci/depot.py --self-test           the gate: no compiler, no network

⚠ This line read *"Package the Linux desktop build"* until 2026-08-14, and it
was quoted as evidence that the depot was Linux-only — while `TARGETS` had
held `win-x86_64` for as long as anyone remembers, four of `--self-test`'s
checks assert the Windows spec and the `.exe` launch contract, and Windows
depots have been published and notarized repeatedly. **A first line is what
everyone reads and nobody re-checks; this one was wrong in the direction that
hides a shipped capability.** The finding is `fix/us-east-shard`'s (2026-08-12)
and sat unmerged on a branch, which is its own version of the same lesson.

A **depot** is one build of one game: a flat list of files each with a sha256,
plus the single command that starts it. The scry launcher installs it by
fetching every file, hashing it, and only then moving the whole thing into
place — so a killed or corrupted install leaves nothing behind. The format and
its rules are `docs/LAUNCHER.md` §3 in the scry repo; this script is Gates' end
of that seam and the launcher needs no knowledge of this game whatsoever.

## Three things this deliberately does NOT do

**It does not compute the depot digest.** The digest is the bytes32 that gets
notarized on chain, and `scrylauncher.depot.digest()` is its one implementation
— a second one here would be exactly the bug scry's invariant 3 is about, with
the added charm that the two would only disagree on some file nobody thought to
test. If a `scry` binary is on PATH this script shells out to `scry digest`;
otherwise it prints the command and stops short of guessing.

**It does not publish.** Publishing is an operator act (`CLAUDE.md`): a build
goes live when the origin's `published.json` names it, and that file is written
by a person who has looked at what they are about to serve. This script prints
the two commands and performs neither.

**It does not bundle SYSTEM libraries** — and the emphasis is load-bearing,
because this line read "system libraries" flatly until 2026-08-16 and that
sentence shipped a Windows build no player could start. The binary loads
libwayland, libudev and libasound from the machine it runs on, and shipping
Ubuntu's copies of those to an arbitrary distro trades a clear error for a
confusing one. What it does instead is *measure* what the binary needs
(`requires.libs`, read off the ELF, never typed) so a launcher can say "you are
missing libasound.so.2" BEFORE the download rather than after a silent failure
to start. That measure exists because this exact wall was hit building it: the
first `--features render` build died in `wayland-sys`'s build script on a box
with no `libwayland-dev`, and a player's box will fail the same way at runtime
with much less to go on.

⚠ **A library the platform does not ship at all is a different question, and
answering it the same way is a game that cannot start.** The mingw build
imports `libstdc++-6.dll` — mingw's C++ runtime, reached through
`basis-universal-sys`, and present on approximately zero stock Windows
machines. It is not the player's to provide; it is ours, and `runtime_dlls`
stages it beside the exe along with everything it transitively needs. The
distinction is asked rather than typed (`dll_probe` in TARGETS), so a name is
sorted by whether the toolchain has a copy of it, not by a list somebody
maintains. The measure was never the problem: `0.2.0-gbed9e02d6` NAMED
`libstdc++-6.dll` in its published `requires.libs` and staged three files.

## The build id

`<version>-g<short sha>`, plus `-dirty` if the tree has uncommitted changes.
Two builds from different commits are different builds and must be, because the
build id is a directory name on the player's disk and the key an install's
receipt is filed under. A dirty tree is *allowed* and *named* — refusing would
just get worked around, and a build id that hides it is worse than one that
says so.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SLUG = "gates"
PLATFORM = "linux-x86_64"
DEPOT_VERSION = 1

# ── the platforms this packager can produce ──────────────────────────────────
# **The tag is the launcher's vocabulary, not ours.** `scry-depot`'s `PLATFORMS`
# is the list a manifest is validated against and Windows is spelled
# `win-x86_64` there — `windows-x86_64` is not a near-miss, it is refused as
# `unknown platform` and the row never renders. The tag is also the key in
# `published.json`, so a wrong one publishes a build nothing will ever ask for.
#
# `triple` is None for the host: no `--target`, so cargo writes to
# `target/release/` rather than `target/<triple>/release/`. Every other field
# exists because it differs per platform and guessing it produces a depot that
# installs and then fails to start — the executable's NAME is part of the
# launch contract, and `strip` is a different binary for a cross target.
TARGETS = {
    "linux-x86_64": {
        "triple": None,
        "exec": "gates",
        "strip": "strip",
        "method": "elf-dt-needed",
        "dlopen": "libxkbcommon-x11, libGL, libvulkan",
    },
    "win-x86_64": {
        "triple": "x86_64-pc-windows-gnu",
        "exec": "gates.exe",
        "strip": "x86_64-w64-mingw32-strip",
        "method": "pe-import-table",
        "dlopen": "vulkan-1.dll, d3d12.dll",
        # **How a DLL name is sorted into "Windows provides this" vs "we do".**
        # `gcc -print-file-name=X` answers with an absolute path when the
        # toolchain has a copy of X and echoes the bare name straight back when
        # it does not, so the question has a file-or-nothing answer and there
        # is no typed list to rot. `kernel32.dll` resolves to nothing;
        # `libstdc++-6.dll` resolves to a path, because it is mingw's C++
        # runtime and not Windows'.
        #
        # It also resolves through the SELECTED mingw alternative, which is the
        # property that matters most: a client linked against `13-posix` cannot
        # be handed `13-win32`'s copy of the same filename. That is the
        # threading-model split `CLAUDE.md` already warns about at build time,
        # arriving again at package time.
        "dll_probe": "x86_64-w64-mingw32-gcc",
        # What the shipped DLLs must be, checked rather than assumed. A 32-bit
        # DLL beside a 64-bit exe is `0xc000007b` on the player's machine.
        "pe_format": "pei-x86-64",
    },
}

# Where the bytes will be fetched from. Baked at package time and NOT rewritten
# by the origin, because the digest is taken over the whole document including
# this field — a server that edited it would change the number a player
# recomputes and looks up on chain.
#
# ⚠ **`{platform}` is in the path, and that is the fix for a real collision.**
# A build id carries no platform, so the origin's old `<slug>/<build>/` was one
# directory for two platforms: the second publish overwrote the first and a
# linux player was handed the windows build. scry keys by (build, platform)
# since 2026-08-14 and this is the packager's half of it.
#
# Because `root` is inside the digest, changing this line changes the number
# notarized for every build packaged after it — which is correct and is why
# depots published BEFORE it keep the old two-segment url forever. The origin
# still serves that shape; a published depot's location is part of what was
# sealed on chain, so it can never be moved.
# ⚠ THE HOST MOVED. `scry.moreright.xyz` was retired 2026-08-20 and now answers
# 410 Gone; the platform is `elopros.com`. A depot packaged with the old root
# names its files on a dead host, which is exactly what happened to
# 0.5.0-gce57499e6 — it installed nowhere until this line was fixed. Only the
# PLATFORM host moved: `game.moreright.xyz` is the public shard and is live.
DEFAULT_ROOT = ("https://elopros.com/api/launcher/depot"
                "/{slug}/{build}/{platform}/files")

# The launcher fills these. `{server}` is the shard to join, `{wallet}` the
# address the player asked their launcher to watch, `{servers}` the url of the
# shard LIST — scry's manifest `servers.url`, the same document its Servers
# window reads. All three arrive as an empty string when the launcher has
# nothing to put there, which `crates/client/src/args.rs` reads as absence —
# that is the normal anonymous launch, not an error.
#
# `{servers}` is why the intro screen was empty over a live shard: the list was
# published and the url was set, and nothing on the argv could carry it, so the
# in-game browser only ever knew the loopback default. It needs a launcher
# holding scry-depot's `ARG_VARS` with "servers" in it — an older one refuses
# the whole launch rather than passing a literal `{servers}` through, which is
# the right failure but a loud one, so this line and that one ship together.
LAUNCH_ARGS = ["--server", "{server}", "--identity", "{wallet}",
               "--servers", "{servers}"]

CHUNK = 262144


def sh(*cmd, cwd=None, check=True) -> str:
    p = subprocess.run(cmd, cwd=cwd or ROOT, text=True, capture_output=True)
    if check and p.returncode != 0:
        sys.exit(f"depot: `{' '.join(cmd)}` failed:\n{p.stderr.strip()}")
    return p.stdout.strip()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for block in iter(lambda: fh.read(CHUNK), b""):
            h.update(block)
    return h.hexdigest()


# ── the build id ─────────────────────────────────────────────────────────────

def crate_version() -> str:
    """The client's version, following `version.workspace = true` to its source.

    The crate carries the inherited form, so a regex for a quoted version in
    `crates/client/Cargo.toml` finds nothing there and the old fallback
    returned `"0.0.0"` — a build id that names no release, written into a
    directory name on a player's disk, with no error anywhere. That is the
    failure this function is now shaped to refuse: it raises rather than
    guessing, because a depot id is not a field worth defaulting.
    """
    for path, why in (
        (ROOT / "crates" / "client" / "Cargo.toml", "the client crate"),
        (ROOT / "Cargo.toml", "the workspace"),
    ):
        txt = path.read_text("utf-8")
        # `version.workspace = true` means "look one level up", so a bare
        # quoted version is the only thing that answers here.
        m = re.search(r'^version\s*=\s*"([^"]+)"', txt, re.M)
        if m:
            return m.group(1)
        del why
    raise SystemExit(
        "ci/depot.py: no version found in crates/client/Cargo.toml or the "
        "workspace Cargo.toml — a depot id must name a real release"
    )


def build_id(binary: Path | None = None) -> str:
    """`<version>-g<short sha>`, plus a dirty marker that is CONTENT-KEYED.

    A build id is a directory name on the player's disk and the key an install
    receipt is filed under, so two different builds must never share one. The
    git sha gives that for a clean tree. It does **not** for a dirty one, and
    this was found by running the packager twice against two genuinely
    different binaries and getting `0.1.0-g6962b4a8-dirty` both times — the
    second would have installed over the first, under a name that claimed they
    were the same build. The digest would have disagreed, so the launcher would
    have called it an update forever rather than silently serving the wrong
    bytes; that is the failure landing in a survivable place, not an argument
    that it is fine.

    So when the tree is dirty the id carries the first 8 hex of the launch
    binary's own sha256. Different bytes, different id, always. A clean tree
    keeps the plain reproducible form.
    """
    ver = crate_version()
    try:
        sha = sh("git", "rev-parse", "--short=9", "HEAD")
        dirty = bool(sh("git", "status", "--porcelain"))
    except SystemExit:
        return ver
    if not dirty:
        return f"{ver}-g{sha}"
    mark = sha256_file(binary)[:8] if binary and binary.is_file() else "unknown"
    return f"{ver}-g{sha}-dirty.{mark}"


# ── what the binary needs from the machine ───────────────────────────────────

def needed_libs(binary: Path, platform: str = PLATFORM) -> tuple[list[str], str]:
    """What the binary asks the machine for. ([libs], why-if-empty).

    Two formats, one idea. On Linux it is the ELF's `DT_NEEDED` sonames; on
    Windows it is the PE import table's `DLL Name:` entries, read with the same
    `objdump` (GNU binutils parses both, so no second tool is owed). The
    incompleteness warning below applies to BOTH and for the same reason —
    an import table is link-time too, and a `LoadLibrary` at runtime appears in
    it exactly as little as a `dlopen` appears in DT_NEEDED.

    Measured with whichever of objdump/readelf is present, and an empty list
    is NOT reported as "needs nothing" — a build tool that is missing and a
    binary that is genuinely static are different facts, and collapsing them
    is the trap scry's CLAUDE.md names (`reachable` is not `empty`).

    ⚠ **This is LINK-TIME only, and the list is therefore incomplete.** The
    depot says so in `requires.complete: false` rather than letting a player
    read it as a checklist. Found the hard way on 2026-08-05: the packaged
    build started, connected to a live shard, and *then* panicked on
    `libxkbcommon-x11.so.0` — which winit `dlopen`s at runtime and which
    appears in no DT_NEEDED entry. The same is true of `libGL` and
    `libvulkan`. Enumerating those would mean either a typed list (which rots
    the first time Bevy changes a backend) or running the binary on a stripped
    machine and watching it fail, which is a different tool than this one.
    What the measured list IS good for is the opposite direction: everything
    on it is definitely required, so a machine missing one of these will
    definitely fail.
    """
    if TARGETS.get(platform, {}).get("method") == "pe-import-table":
        probes = ((["objdump", "-p", str(binary)], r"^\s*DLL Name:\s*(\S+)"),)
        absent = ("objdump is not installed on the build machine, so the DLLs "
                  "this build imports were NOT measured")
    else:
        probes = (
            (["objdump", "-p", str(binary)], r"^\s*NEEDED\s+(\S+)"),
            (["readelf", "-d", str(binary)], r"\(NEEDED\).*\[([^\]]+)\]"),
        )
        absent = ("neither objdump nor readelf is installed on the build machine, "
                  "so the shared libraries this build needs were NOT measured")
    for cmd, pat in probes:
        if shutil.which(cmd[0]) is None:
            continue
        p = subprocess.run(cmd, text=True, capture_output=True)
        if p.returncode != 0:
            continue
        libs = sorted(set(re.findall(pat, p.stdout, re.M)))
        return libs, ""
    return [], absent


def toolchain_dll(name: str, probe: str) -> Path | None:
    """The toolchain's own copy of `name`, or None when it has none.

    None is the answer for every genuine Windows DLL — `kernel32.dll`,
    `ws2_32.dll`, `vulkan-1.dll` — and a path is the answer for the mingw
    runtime. See `dll_probe` in TARGETS for why this is asked and not typed.
    """
    if shutil.which(probe) is None:
        return None
    p = subprocess.run([probe, f"-print-file-name={name}"],
                       text=True, capture_output=True)
    if p.returncode != 0:
        return None
    got = Path(p.stdout.strip())
    return got if got.is_absolute() and got.is_file() else None


def pe_format(path: Path) -> str:
    """objdump's name for a PE's machine type, or "" if it could not be read."""
    if shutil.which("objdump") is None:
        return ""
    p = subprocess.run(["objdump", "-f", str(path)], text=True, capture_output=True)
    m = re.search(r"file format (\S+)", p.stdout) if p.returncode == 0 else None
    return m.group(1) if m else ""


def runtime_dlls(binary: Path, platform: str) -> list[Path]:
    """Every DLL this build needs that Windows will not provide, transitively.

    **This exists because the depot shipped a build that could not start.**
    `0.2.0-gbed9e02d6`'s own measured `requires.libs` named `libstdc++-6.dll`
    among two dozen genuine system DLLs, the staged tree held exactly three
    files (`gates.exe`, `LICENSE`, `NOTICE`), and no gate could see the gap:
    the packager hashed it, the launcher installed it, and the player got an
    Application Error box before a frame was drawn. The Linux rule this
    packager was written under — bundle nothing, the distro provides it — does
    not transfer, because mingw's C++ runtime is not a thing a Windows machine
    has. It is OUR runtime, and it rides with the binary that needs it.

    **The closure is the whole point and a direct read of the exe misses it.**
    `gates.exe` names `libstdc++-6.dll` and nothing else of ours; that DLL in
    turn imports `libgcc_s_seh-1.dll` and `libwinpthread-1.dll`, which appear
    in the executable's import table nowhere. Shipping only what the exe names
    would produce a build that still cannot start, and the second failure would
    be indistinguishable from the first.

    Refusing is better than shipping a partial answer, so a missing probe or a
    missing objdump exits rather than returning an empty list: staging no DLLs
    at all is precisely the defect, and it must not be reachable by a tool
    being absent.
    """
    spec = TARGETS[platform]
    probe = spec.get("dll_probe")
    if not probe:
        return []
    if shutil.which(probe) is None:
        sys.exit(f"depot: {probe} is not on this machine, so the DLLs this build "
                 "needs cannot be resolved. Refusing to stage a Windows build "
                 "with no runtime beside it — that ships a game nobody can start.")

    found: dict[str, Path] = {}
    queue, seen = [binary], set()
    while queue:
        imports, why = needed_libs(queue.pop(), platform)
        if why:
            sys.exit(f"depot: {why}. Refusing to stage a Windows build whose "
                     "runtime DLLs were never measured.")
        for name in imports:
            key = name.lower()
            if key in seen:
                continue
            seen.add(key)
            src = toolchain_dll(name, probe)
            if src is None:
                continue          # Windows' to provide, not ours.
            want = spec.get("pe_format")
            got = pe_format(src)
            if want and got != want:
                sys.exit(f"depot: {src} is {got or 'unreadable'}, not {want}. "
                         "Shipping it would be 0xc000007b on the player's machine.")
            found[key] = src
            queue.append(src)     # ...and whatever IT needs.
    return [found[k] for k in sorted(found)]


# ── the depot document ───────────────────────────────────────────────────────

def safe_relpath(raw: str) -> str:
    """The launcher's own path rule, restated at the writing end.

    Refusing here is worth more than refusing at install time: a depot that
    names a path the launcher will reject is a build that cannot be installed
    by anyone, and the person who can fix it is standing right here.
    """
    if not raw or "\x00" in raw or "\\" in raw or len(raw) > 1024:
        sys.exit(f"depot: bad path {raw!r}")
    if raw.startswith("/") or raw.endswith("/") or (len(raw) > 1 and raw[1] == ":"):
        sys.exit(f"depot: paths must be relative files: {raw!r}")
    parts = raw.split("/")
    if any(p in ("", ".", "..") for p in parts):
        sys.exit(f"depot: path escapes the build directory: {raw!r}")
    return raw


def build_depot_doc(stage: Path, build: str, root: str, *,
                    exec_name: str = "gates",
                    platform: str = PLATFORM,
                    executables: set[str] | None = None,
                    requires: dict | None = None) -> dict:
    """Walk a staged build directory and produce the depot index.

    Pure apart from reading the staged files, so `--self-test` can drive it
    with a handful of fake ones and assert the rules without a compiler.
    """
    executables = executables if executables is not None else {exec_name}
    files, seen = [], set()
    for p in sorted(stage.rglob("*")):
        if not p.is_file() or p.is_symlink():
            continue
        rel = safe_relpath(p.relative_to(stage).as_posix())
        if rel.lower() in seen:
            sys.exit(f"depot: two files differ only by case: {rel!r}")
        seen.add(rel.lower())
        files.append({
            "path": rel,
            "sha256": sha256_file(p),
            "bytes": p.stat().st_size,
            "executable": rel in executables,
        })
    if not files:
        sys.exit(f"depot: nothing staged in {stage}")
    if exec_name not in {f["path"] for f in files}:
        # The launcher re-checks this at launch time too, on purpose. Catching
        # it here means the mistake never reaches a player's disk.
        sys.exit(f"depot: launch.exec {exec_name!r} is not one of the staged files")

    doc = {
        "depot_version": DEPOT_VERSION,
        "slug": SLUG,
        "build": build,
        "platform": platform,
        "root": root.format(slug=SLUG, build=build, platform=platform),
        "files": files,
        "launch": {
            "exec": exec_name,
            "args": LAUNCH_ARGS,
            "cwd": ".",
            # No LD_LIBRARY_PATH: nothing is bundled, so pointing the loader
            # anywhere would only shadow the system's own copies. The launcher
            # would allow one as long as it resolved inside the build; there is
            # simply nothing for it to resolve to.
            "env": {},
        },
    }
    if requires:
        doc["requires"] = requires
    return doc


# ── staging ──────────────────────────────────────────────────────────────────

def stage_build(out: Path, *, platform: str, do_build: bool,
                do_strip: bool) -> tuple[Path, dict]:
    """Compile (optionally), strip, and stage into `<out>/.staging`.

    The staging directory is deliberately NOT named for the build: the build id
    for a dirty tree is keyed on the staged binary's hash, so the name cannot
    be known until after the strip. `main` renames once it is.
    """
    spec = TARGETS[platform]
    triple, exec_name = spec["triple"], spec["exec"]
    cmd = ["cargo", "build", "--release", "-p", "client", "--features", "render",
           "--bin", "gates"]
    if triple:
        cmd += ["--target", triple]
    if do_build:
        print("== " + " ".join(cmd))
        p = subprocess.run(cmd, cwd=ROOT, text=True)
        if p.returncode != 0:
            sys.exit("depot: the build failed — nothing staged")

    # cargo writes a cross build under `target/<triple>/release/`, the host
    # build straight into `target/release/`. Reading the wrong one is how a
    # "windows depot" ends up holding the linux binary under a .exe name.
    built = ROOT / "target" / (f"{triple}/release" if triple else "release") / exec_name
    if not built.is_file():
        sys.exit(f"depot: {built} is not there. Drop --no-build, or build it first.")
    binary = built

    libs, lib_why = needed_libs(binary, platform)

    stage = out / ".staging"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    dest = stage / exec_name
    shutil.copy2(binary, dest)

    # **The runtime the player's machine will not have.** Beside the exe, not
    # in a subdirectory: Windows searches the directory the application loaded
    # from FIRST, ahead of System32 and ahead of PATH, so a co-located copy is
    # both the one that resolves and the one that wins over whatever stray
    # same-named DLL a player already has. That second half is not incidental —
    # a 32-bit `libstdc++-6.dll` on a player's PATH is `0xc000007b` rather than
    # a missing-DLL message, and the same file placed here shadows it.
    #
    # Nothing is added to `launch.env` for this and nothing should be: the
    # application directory is already first in the search order, and a build
    # that needed an environment variable to find its own runtime would be one
    # more thing to get wrong.
    runtime = runtime_dlls(binary, platform)
    for src in runtime:
        shutil.copy2(src, stage / src.name)
    bundled = sorted(p.name for p in runtime)
    if bundled:
        print(f"   staged {len(bundled)} runtime DLL(s): {', '.join(bundled)}")
        # **Bundling them creates a notice obligation that measuring them did
        # not.** `libwinpthread-1.dll` is Expat/BSD — notice licences, where
        # the credit IS the condition — and the two GCC DLLs are GPLv3 carried
        # by the Runtime Library Exception, which is the term that lets a sold
        # MIT game redistribute them at all. Naming that reliance is the point:
        # `CLAUDE.md`'s rail says nothing here derives from GPL work, and this
        # is the one place that needs a stated reason rather than a silence.
        # Fatal, exactly like LICENSE and NOTICE below, and for the same reason
        # — a depot that plays while missing a licence it must carry is a
        # defect that never arrives as a bug report.
        notice_src = ROOT / "ci" / "licenses" / "MINGW-RUNTIME.txt"
        if not notice_src.is_file():
            sys.exit(f"depot: {notice_src} is missing — the runtime DLLs may not "
                     "ship without the notice that has to travel with them")
        shutil.copy2(notice_src, stage / "LICENSE-MINGW-RUNTIME.txt")
        print("   staged LICENSE-MINGW-RUNTIME.txt (their notice travels with them)")

    # **The assets ride along, and forgetting them would not look like an
    # error.** `crates/client/src/render/textures.rs` loads
    # `textures/<role>_{albedo,normal,rough}.jpg` at runtime, and Bevy answers
    # a missing texture with a white fallback and a log line while the
    # renderer carries on drawing — the exact failure `gates.rs` documents
    # (three material changes measuring byte-identical because the path was
    # wrong). A depot of the binary alone would install perfectly, start
    # perfectly, and render an untextured world.
    #
    # They land at `assets/` inside the build directory, which is where the
    # client looks: the launcher runs a build with cwd set to that directory,
    # and `gates.rs` resolves its asset root against the cwd.
    # ⚠ **Only what git TRACKS, and never a filesystem walk.** This was
    # `shutil.copytree(src_assets, staged_assets)` until 2026-08-14, and that
    # walk staged `assets/textures/candidates/` — the 1.3 GB sourcing queue
    # that commit 36776f4 deliberately keeps out of the tree ("its 1.3 GB does
    # not"), 190 files of raw CC0/CC-BY archives fetched to this box. The
    # depot went 124 MB → 1.6 GB, 122 files → 317, and **nothing failed**:
    # `ci/gates.sh` was green, `--self-test` was green, the document validated.
    # An untracked file is invisible to every gate in this repo, so no gate
    # could have caught it — which is why the rule has to be structural rather
    # than a check. `.gitignore` is already the single author of what is not
    # ours to ship; reading it here means the packager cannot disagree with it.
    # scry's `deploy/publish_scryward.py` makes the same choice for the same
    # reason, and states it as the trap it has paid for seven times.
    #
    # It also has teeth beyond size: the candidates are *unvetted* licence
    # material (`CANDIDATES.md` carries CC-BY rows with draft notices), and the
    # rail in `assets/models/MANIFEST.md` governs what SHIPS. A filesystem walk
    # routes around that rail without anyone deciding to.
    src_assets = ROOT / "assets"
    if not src_assets.is_dir():
        sys.exit(f"depot: {src_assets} is missing — the client would ship untextured")
    staged_assets = stage / "assets"
    rels = [r for r in sh("git", "ls-files", "-z", "--", "assets").split("\0") if r]
    if not rels:
        sys.exit(
            "depot: `git ls-files assets` listed nothing. Refusing to fall back "
            "to a filesystem walk — that is the 1.6 GB defect this refusal "
            "exists to prevent. Package from a git checkout."
        )
    for rel in rels:
        src = ROOT / rel
        if not src.is_file():
            sys.exit(f"depot: {rel} is tracked but not on disk — the tree is incomplete")
        dst = stage / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
    n_assets = sum(1 for p in staged_assets.rglob("*") if p.is_file())
    if n_assets == 0:
        sys.exit(f"depot: {src_assets} holds no files — the client would ship untextured")
    skipped = sum(1 for p in src_assets.rglob("*") if p.is_file()) - n_assets
    print(f"   staged {n_assets} tracked asset file(s) from {src_assets}"
          + (f" ({skipped} untracked file(s) left behind)" if skipped else ""))

    # **The licence rides along too, and this one is a legal condition rather
    # than a rendering one.** MIT requires its notice to accompany "all copies
    # or substantial portions", and a depot installed by the launcher IS a
    # copy — arguably the main one, since it is how a player receives the
    # game. `NOTICE` carries the third-party credits that are themselves
    # conditions: the icon set is CC BY and the UI face is Apache-2.0, and for
    # both of those the attribution is what the licence is *for*.
    #
    # `assets/icons/CREDITS.md` already rides inside `assets/` above, which
    # covers the icons on its own; these two are the root-level files that
    # would otherwise be left behind, and were, until 2026-08-11.
    #
    # Fatal rather than best-effort. A depot that installs and plays while
    # missing a licence it is required to carry is exactly the kind of defect
    # that never surfaces as a bug report.
    for name in ("LICENSE", "NOTICE"):
        src = ROOT / name
        if not src.is_file():
            sys.exit(f"depot: {src} is missing — a build may not ship without its licence")
        shutil.copy2(src, stage / name)
    print("   staged LICENSE + NOTICE")

    # The cross target gets the CROSS strip. The host's `strip` on a PE is not
    # a no-op that leaves a fat binary — binutils either refuses the format or
    # rewrites it, and a corrupted .exe passes every check this packager makes
    # (it hashes, it stages, it installs) and dies on the player's machine.
    stripper = spec["strip"]
    if do_strip and shutil.which(stripper):
        before = dest.stat().st_size
        subprocess.run([stripper, str(dest)], check=False)
        print(f"   stripped {before:,} -> {dest.stat().st_size:,} bytes")
    elif do_strip:
        print(f"   NOT stripped — {stripper} is not on this machine")
    dest.chmod(0o755)

    return stage, requires_doc(libs, bundled, spec, lib_why)


def requires_doc(libs: list[str], bundled: list[str], spec: dict,
                 lib_why: str = "") -> dict:
    """The `requires` block: what the player's machine still has to provide.

    Pure, so `--self-test` can drive it with no compiler and no binary — the
    subtraction below is the part that would otherwise only be checked by a
    person reading a published depot.
    """
    # **`requires.libs` is what the PLAYER's machine must have**, so anything
    # riding in the depot comes back out of it. A launcher that told a player
    # to go install `libstdc++-6.dll` would be sending them after a file we
    # just handed them, and the one real gap would be buried in that noise.
    # Case-insensitively, because a PE import table is: `KERNEL32.dll` and
    # `kernel32.dll` both appear in one real measurement of this binary.
    shipped = {b.lower() for b in bundled}
    requires = {
        "libs": [lib for lib in libs if lib.lower() not in shipped],
        "method": spec["method"],
        # The honest bound on the list above. Everything in it IS required;
        # it is not everything that is required — winit and wgpu load the
        # graphics stack at runtime, and those appear in no link-time entry on
        # either platform. A launcher may use this to say "you are definitely
        # missing X"; it may never say "you have everything".
        "complete": False,
        "why": ("loaded from the player's machine at start. No SYSTEM library is "
                "bundled: shipping one machine's copies of those to another trades "
                "a clear error for a confusing one. What the platform does not "
                "provide at all rides in the depot instead and is listed under "
                "`bundled` — the two are different questions and answering them "
                "the same way is what shipped a Windows build that could not "
                "start. LINK-TIME entries only — libraries opened at runtime "
                f"({spec['dlopen']}) are not listed and this is not a checklist."),
    }
    # Named, not just staged. A player reporting a start-up failure and an
    # operator reading a depot both need to know which runtime this build was
    # cut against, and the file list alone does not say that these are the C++
    # runtime rather than game data.
    if bundled:
        requires["bundled"] = bundled
    if lib_why:
        requires["measured"] = False
        requires["why"] = lib_why
    else:
        requires["measured"] = True
    return requires


# ── the gate ─────────────────────────────────────────────────────────────────

def self_test() -> int:
    """No compiler, no network, no cargo. Runs in ci/gates.sh.

    What it guards is the document, not the build: a depot whose paths, launch
    block or hashes are wrong is a game nobody can install, and the mistake
    would otherwise only surface on a player's machine.
    """
    passed = 0

    def ok(cond, label):
        nonlocal passed
        if not cond:
            print(f"FAIL: {label}")
            sys.exit(1)
        passed += 1
        print(f"  ok: {label}")

    tmp = Path(tempfile.mkdtemp(prefix="gates-depot-selftest-"))
    stage = tmp / "0.1.0-gdeadbeef"
    (stage / "lib").mkdir(parents=True)
    (stage / "gates").write_bytes(b"\x7fELF pretend\n")
    (stage / "lib" / "extra.pak").write_bytes(b"payload\n")

    doc = build_depot_doc(stage, "0.1.0-gdeadbeef", DEFAULT_ROOT,
                          requires={"libs": ["libc.so.6"], "measured": True})

    ok(doc["depot_version"] == DEPOT_VERSION, "the depot declares its version")
    ok(doc["slug"] == SLUG and doc["platform"] == PLATFORM, "slug and platform")
    ok(len(doc["files"]) == 2, "every staged file is listed")
    ok([f["path"] for f in doc["files"]] == ["gates", "lib/extra.pak"],
       "paths are relative, posix, and sorted")
    ok(all(len(f["sha256"]) == 64 for f in doc["files"]), "each carries a sha256")
    ok(doc["files"][0]["sha256"] == hashlib.sha256(b"\x7fELF pretend\n").hexdigest(),
       "and the sha256 is of the real bytes")
    ok(doc["files"][0]["bytes"] == 13, "and the real size")
    ok(doc["files"][0]["executable"] is True, "the launch binary is executable")
    ok(doc["files"][1]["executable"] is False,
       "and nothing else is — a file is executable iff the depot says so")

    ok(doc["launch"]["exec"] == "gates", "launch.exec names a staged file")
    ok("{server}" in doc["launch"]["args"], "the launch args carry {server}")
    ok("{wallet}" in doc["launch"]["args"], "and {wallet}")
    ok("{servers}" in doc["launch"]["args"],
       "and {servers} — without it the in-game browser is empty over a live shard")
    ok(doc["launch"]["env"] == {}, "nothing is bundled, so nothing redirects the loader")
    ok("{build}" not in doc["root"] and doc["root"].startswith("https://"),
       "the root is filled in and https")
    ok("0.1.0-gdeadbeef" in doc["root"], "and it points at THIS build")
    # The platform belongs in the PATH, not just in the document's `platform`
    # field: the origin keys by (build, platform), and a root without it sends
    # both platforms of one commit to one directory — which is the collision
    # that hands a linux player the windows build.
    ok("{platform}" not in doc["root"] and f"/{doc['platform']}/" in doc["root"],
       "and at THIS platform — the pair is the origin's key")

    # The placeholders must be ones the launcher knows. An unknown one is a
    # refusal at launch, not a passthrough, so a typo here would ship a build
    # that installs perfectly and never starts.
    known = {"server", "wallet", "build_dir", "host", "servers"}
    used = {m for a in doc["launch"]["args"] for m in re.findall(r"\{([a-z_]+)\}", a)}
    ok(used <= known, f"every placeholder is one the launcher fills: {sorted(used)}")

    # The path rules, at the writing end.
    for bad in ("../escape", "/abs", "a//b", "./a", "a\\b", "x\x00y", ""):
        try:
            safe_relpath(bad)
            ok(False, f"safe_relpath should refuse {bad!r}")
        except SystemExit:
            ok(True, f"refused as a depot path: {bad!r}")

    # A staged tree with no launch binary must not produce a document.
    empty = tmp / "0.0.0"
    (empty).mkdir()
    (empty / "notgates").write_bytes(b"x")
    try:
        build_depot_doc(empty, "0.0.0", DEFAULT_ROOT)
        ok(False, "a stage without launch.exec should be refused")
    except SystemExit:
        ok(True, "a stage without launch.exec is refused before it ships")

    # The build id: shape, and the property that matters.
    bid = build_id(stage / "gates")
    ok(re.match(r"^\d+\.\d+\.\d+(-g[0-9a-f]+)?(-dirty\.[0-9a-f]{8}|-dirty\.unknown)?$", bid),
       f"build id shape: {bid}")
    ok(all(c.isalnum() or c in "-_." for c in bid) and not bid.startswith("."),
       "a build id is a safe directory name and a safe url segment")

    # The defect this test exists for: two different binaries under one id.
    # A build id is a directory name on a player's disk and the key an install
    # receipt is filed under, so a collision means one build installing over
    # another under a name claiming they are the same.
    other = tmp / "other"
    other.mkdir()
    (other / "gates").write_bytes(b"\x7fELF a DIFFERENT build\n")
    if "-dirty" in bid:
        ok(build_id(other / "gates") != bid,
           "two different binaries from one dirty tree get DIFFERENT build ids")
    else:
        # A clean tree is reproducible by construction — the id is the commit,
        # and two different binaries from one commit is a toolchain question,
        # not something a name can fix.
        ok(build_id(other / "gates") == bid,
           "a clean tree's id is the commit and does not move with the bytes")

    # The assets the shipped client loads at runtime must exist to be staged.
    # Checked here rather than only at package time because the failure is
    # invisible downstream: Bevy answers a missing texture with a white
    # fallback and keeps drawing, so a depot without them installs perfectly,
    # starts perfectly, and renders an untextured world.
    src_assets = ROOT / "assets"
    ok(src_assets.is_dir(), f"{src_assets} exists to be staged into the depot")
    tex = sorted((src_assets / "textures").glob("*_albedo.jpg"))
    ok(len(tex) > 0, f"and carries the textures the render path loads ({len(tex)} albedo)")

    # The licence a depot is REQUIRED to carry, for the same reason and with a
    # harder edge: MIT's notice must accompany every copy, and a depot is how
    # a player receives this game. Checked here — no compiler, no network — so
    # a deleted or renamed licence reddens `ci/gates.sh` rather than shipping.
    for name in ("LICENSE", "NOTICE"):
        ok((ROOT / name).is_file(), f"{name} exists to be staged into the depot")
    lic = (ROOT / "LICENSE").read_text("utf-8")
    ok("MIT License" in lic and "Copyright (c)" in lic,
       "LICENSE is the MIT text with a copyright line")
    # `Cargo.toml` states the licence in metadata and `LICENSE` is the grant
    # itself; the two disagreeing is how a repo ends up promising one thing in
    # its manifest and another in its file.
    ws = (ROOT / "Cargo.toml").read_text("utf-8")
    ok('license = "MIT"' in ws, "the workspace manifest agrees with LICENSE")
    notice = (ROOT / "NOTICE").read_text("utf-8")
    # The two notice licences whose credit IS the condition. Named rather than
    # counted: a count drifts every time an icon lands, a name does not.
    for who in ("game-icons.net", "CC BY 3.0", "Roboto Condensed", "Apache-2.0"):
        ok(who in notice, f"NOTICE still credits {who}")
    # The Windows depot redistributes three binaries that are not ours, one of
    # them GPL-licensed. Two notice licences and one exception being relied on
    # is the most load-bearing credit in this file, and the only one whose
    # absence would be a licence breach in a SOLD product.
    lic = ROOT / "ci" / "licenses" / "MINGW-RUNTIME.txt"
    ok(lic.is_file(), "the mingw runtime notice exists to be staged beside the DLLs")
    mingw = lic.read_text("utf-8")
    for who in ("libstdc++-6.dll", "libgcc_s_seh-1.dll", "libwinpthread-1.dll",
                "GCC Runtime Library Exception", "mingw-w64"):
        ok(who in mingw, f"the mingw runtime notice names {who}")
    ok("GCC Runtime Library Exception" in notice and "mingw-w64" in notice.lower(),
       "NOTICE names the mingw runtime and the exception that permits shipping it")

    # A nested asset path survives the document writer unchanged — the shape
    # `assets/textures/x.jpg` takes.
    nested = tmp / "nested"
    (nested / "assets" / "textures").mkdir(parents=True)
    (nested / "gates").write_bytes(b"x")
    (nested / "assets" / "textures" / "rock_albedo.jpg").write_bytes(b"jpeg")
    ndoc = build_depot_doc(nested, "0.0.0", DEFAULT_ROOT)
    ok([f["path"] for f in ndoc["files"]] == ["assets/textures/rock_albedo.jpg", "gates"],
       "nested asset paths are listed relative and posix, sorted")
    ok(all(not f["executable"] for f in ndoc["files"] if f["path"] != "gates"),
       "an asset is never marked executable")

    # ── the platform table ───────────────────────────────────────────────────
    # These tags are not ours to spell. `scry-depot`'s PLATFORMS is what a
    # manifest is validated against and what `platform_tag()` computes on the
    # player's machine, so a tag that is merely *plausible* — `windows-x86_64`,
    # `win64` — is refused as `unknown platform` and the row never renders.
    # Pinned literally here because the failure is silent at this end: this
    # packager would happily write one and the depot would look perfect.
    ok(set(TARGETS) == {"linux-x86_64", "win-x86_64"},
       "the platform tags are the launcher's own vocabulary, spelled its way")
    for tag, spec in TARGETS.items():
        ok({"triple", "exec", "strip", "method", "dlopen"} <= set(spec),
           f"{tag}: the target spec is complete")
    ok(TARGETS["win-x86_64"]["exec"].endswith(".exe"),
       "a windows build launches a .exe — the name is part of the launch contract")
    # **The Windows target must be able to answer "is this DLL mine to ship".**
    # Without a probe, `runtime_dlls` returns nothing and the depot goes out
    # with no C++ runtime beside the exe — which is exactly what `0.2.0` did,
    # and it installs and hashes and verifies perfectly on the way to an
    # Application Error box. Asserted here because the absence is silent.
    ok(TARGETS["win-x86_64"].get("dll_probe"),
       "the windows target knows how to resolve a DLL to the toolchain's copy")
    ok(TARGETS["win-x86_64"].get("pe_format") == "pei-x86-64",
       "and what a shipped DLL must be — a 32-bit one is 0xc000007b on a player's box")
    ok("dll_probe" not in TARGETS["linux-x86_64"],
       "the linux build bundles nothing: the distro provides its system libraries")
    ok(TARGETS["linux-x86_64"]["triple"] is None,
       "the host build takes no --target, so it reads target/release/")

    # A windows depot names the windows platform and the windows executable.
    # Both travel into the digest, so getting either wrong is not cosmetic.
    wstage = tmp / "win"
    wstage.mkdir(parents=True)
    (wstage / "gates.exe").write_bytes(b"MZ")
    wdoc = build_depot_doc(wstage, "0.0.0", DEFAULT_ROOT,
                           exec_name="gates.exe", platform="win-x86_64")
    ok(wdoc["platform"] == "win-x86_64", "a windows depot says win-x86_64")
    ok(wdoc["launch"]["exec"] == "gates.exe", "and launches gates.exe")
    ok([f["path"] for f in wdoc["files"]] == ["gates.exe"] and
       wdoc["files"][0]["executable"],
       "and marks the .exe executable")

    # ── what the player still has to provide ─────────────────────────────────
    # The bug this guards: `0.2.0-gbed9e02d6`'s published `requires.libs` named
    # `libstdc++-6.dll` beside two dozen genuine Windows DLLs, and the depot
    # staged three files. The document was *correct* and the build could not
    # start — measuring a need is not meeting it. Now a bundled DLL leaves the
    # player's list and is named under `bundled` instead.
    wspec = TARGETS["win-x86_64"]
    measured = ["KERNEL32.dll", "libstdc++-6.dll", "ws2_32.dll"]
    req = requires_doc(measured, ["libgcc_s_seh-1.dll", "libstdc++-6.dll",
                                  "libwinpthread-1.dll"], wspec)
    ok("libstdc++-6.dll" not in req["libs"],
       "a DLL the depot ships is not also demanded of the player's machine")
    ok(req["libs"] == ["KERNEL32.dll", "ws2_32.dll"],
       "and the genuine system DLLs stay demanded, in order")
    ok(req["bundled"] == ["libgcc_s_seh-1.dll", "libstdc++-6.dll",
                          "libwinpthread-1.dll"],
       "what rides along is named, not merely staged")
    # The transitive two. They appear in no import table of `gates.exe` — only
    # in `libstdc++-6.dll`'s own — so a packager that read the exe and stopped
    # would ship one third of the runtime and fail identically.
    ok({"libgcc_s_seh-1.dll", "libwinpthread-1.dll"} <= set(req["bundled"]),
       "libstdc++'s own dependencies ride too — the closure, not the direct read")
    ok("Nothing is bundled" not in req["why"],
       "and the document does not claim otherwise")
    # Case, because a PE import table mixes it: one real measurement of this
    # binary carries both `KERNEL32.dll` and `kernel32.dll`.
    ok(requires_doc(["LIBSTDC++-6.DLL"], ["libstdc++-6.dll"], wspec)["libs"] == [],
       "the subtraction is case-insensitive, as DLL names are")
    # Linux is untouched by all of it: nothing is bundled, so nothing is
    # subtracted and no `bundled` key appears.
    lreq = requires_doc(["libc.so.6"], [], TARGETS["linux-x86_64"])
    ok(lreq["libs"] == ["libc.so.6"] and "bundled" not in lreq,
       "a linux depot bundles nothing and demands its full measured list")

    # The depot must serialise to JSON with no surprises — it is fetched and
    # parsed by two independent implementations (python and rust).
    blob = json.dumps(doc, indent=2)
    ok(json.loads(blob) == doc, "the document round-trips through JSON")
    ok(not any(isinstance(f["bytes"], bool) for f in doc["files"]),
       "sizes are ints, not bools")
    ok(all(not isinstance(v, float) for f in doc["files"] for v in f.values()),
       "no floats anywhere — python and rust disagree on how to print some of them, "
       "and a digest that silently differs is the expensive failure")

    shutil.rmtree(tmp, ignore_errors=True)
    print(f"\nPASS — {passed} checks")
    return 0


# ── main ─────────────────────────────────────────────────────────────────────

def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", default=DEFAULT_ROOT,
                    help="where the bytes will be fetched from; {slug}, {build} "
                         "and {platform} are filled in. BAKED INTO THE DIGEST — "
                         "changing it changes the number that gets notarized")
    ap.add_argument("--out", default=str(ROOT / "target" / "depot"),
                    help="where the staged build tree is written")
    ap.add_argument("--platform", default=PLATFORM, choices=sorted(TARGETS),
                    help="which platform to package. The tag is the launcher's "
                         "own vocabulary and the key in published.json")
    ap.add_argument("--build", default=None, help="override the build id")
    ap.add_argument("--no-build", action="store_true", help="package what is compiled")
    ap.add_argument("--no-strip", action="store_true", help="keep debug symbols")
    ap.add_argument("--self-test", action="store_true", help="the gate; no compiler")
    a = ap.parse_args()

    if a.self_test:
        return self_test()

    out = Path(a.out) / a.platform if a.platform != PLATFORM else Path(a.out)
    exec_name = TARGETS[a.platform]["exec"]
    print(f"== depot {SLUG} ({a.platform})")

    stage, requires = stage_build(out, platform=a.platform,
                                  do_build=not a.no_build,
                                  do_strip=not a.no_strip)

    # The id comes AFTER staging: a dirty tree keys it on the staged binary's
    # own hash, so it cannot be known before the strip.
    build = a.build or build_id(stage / exec_name)
    final = out / build
    if final.exists():
        shutil.rmtree(final)
    stage.rename(final)
    stage = final
    print(f"   build {build}")
    if "-dirty." in build:
        print("   NOTE: the tree has uncommitted changes. The id carries the "
              "binary's own hash so two dirty builds cannot collide.")

    doc = build_depot_doc(stage, build, a.root, exec_name=exec_name,
                          platform=a.platform, requires=requires)
    index = stage / "depot.json"
    index.write_text(json.dumps(doc, indent=2) + "\n", "utf-8")

    total = sum(f["bytes"] for f in doc["files"])
    print(f"\n   {len(doc['files'])} file(s), {total:,} bytes")
    print(f"   {index}")
    if requires.get("measured"):
        print(f"   needs from the machine: {', '.join(requires['libs']) or 'nothing'}")
    else:
        print(f"   shared libraries NOT measured — {requires['why']}")

    # The digest is scry's to compute, not ours. One implementation, and this
    # is not it (scry CLAUDE.md invariant 3).
    print()
    if shutil.which("scry"):
        digest = sh("scry", "digest", str(index), check=False)
        print(f"   digest {digest}" if digest else
              "   `scry digest` produced nothing — run it by hand")
    else:
        print(f"   digest: run `scry digest {index}` (no `scry` on PATH here).")
        print("   This script does NOT compute it — one implementation, and it lives "
              "in the launcher.")

    print("\n== to publish (OPERATOR ACT — read the tree first)")
    print(f"   rsync -a {stage}/ "
          f"<origin>:/data/apps/scry-data/depots/{SLUG}/{build}/{a.platform}/")
    # **A MERGE, not an overwrite.** `published.json` holds one row per
    # platform, so the obvious `echo '{...}' >` publishes this build and
    # silently UNPUBLISHES every other platform — the linux row disappears the
    # moment a windows build is published, and nothing reports it because an
    # absent row is the same shape as a title that has never shipped one.
    print(f"   python3 -c \"import json,pathlib;"
          f"p=pathlib.Path('/data/apps/scry-data/depots/{SLUG}/published.json');"
          f"d=json.loads(p.read_text()) if p.exists() else {{}};"
          f"d['{a.platform}']='{build}';"
          f"p.write_text(json.dumps(d))\"   # on <origin>")
    print("   ...and notarize the digest with ScryNotary on 4663.")
    print("   Until published.json names it, the manifest's native row stays a slot.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
