#!/bin/bash
# Certbot deploy hook: refresh the copies of the shard's chain that the Gates
# shard reads, and restart it so the new one is served.
#
#   sudo cp ops/certbot-deploy-hook.sh \
#           /etc/letsencrypt/renewal-hooks/deploy/copy-gates-cert.sh
#   sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/copy-gates-cert.sh
#
# `ci/deploy_shard.sh` installs it. Everything under `renewal-hooks/deploy/`
# runs after a successful renewal, with `RENEWED_LINEAGE` naming the one that
# moved.
#
# ## Why this file has to exist
#
# **The shard reads the PEMs once, at boot, and never again.** That is stated
# in `shard-public.toml` and in `NETCODE.md` §2.2's prod-certs row, and both
# said the same thing about it for over a week: *"a renewal hook is worth
# writing before this matters."* It matters on the renewal, which for the
# chain in place as this was written is around **2026-09-23** (notAfter
# 2026-10-23; certbot renews at 30 days).
#
# Without this, the failure is quiet and badly disguised: certbot renews, the
# copies under /home/master/gates-certs stay at the old bytes, and the shard
# keeps serving a chain that expires under it. Every joiner then fails the
# handshake with a TLS error that reads exactly like a client bug — and the
# client is right to refuse, because it validates against the platform root
# store on a non-loopback address with no pin (`tls_posture.rs`).
#
# ## Why it is a SEPARATE file from copy-game-cert.sh
#
# There is already a deploy hook on this box for the *other* game server on
# this domain (thrml's — it copies into /data/apps/secrets and pm2-restarts
# it). This one is deliberately not an edit of that: the two games share a
# certificate and nothing else, and a hook that restarts both is a hook where
# one game's bad deploy takes the other down. Certbot runs every executable
# in the directory, so two files is the supported shape and not a workaround.
set -e

# **The name players dial** — `shards.toml`'s `addr` and `shard-public.toml`'s
# `domain`, which are the same string by law. This is what the copied chain
# has to be able to prove, and checking it is the whole point of the two
# blocks below.
SHARD_NAME=game.elopros.com

# **The LINEAGE is resolved, not typed**, because it is a directory name on
# somebody else's box and there are two reasonable ways to have made it. The
# platform moved to elopros.com on 2026-08-20 and the shard followed on
# 08-23; an operator either expanded the existing lineage (`certbot certonly
# --expand -d game.moreright.xyz -d game.elopros.com`, which keeps the old
# path) or issued a fresh one under the new name. Typing either would make
# this hook exit 0 and do nothing on the box that took the other, which is
# exactly the silent failure this file exists to prevent.
#
# So: take the first candidate that exists AND covers `$SHARD_NAME`.
DEST=/home/master/gates-certs
LIVE=
# ⚠ **`openssl x509 -checkhost` EXITS 0 EITHER WAY.** It reports the verdict
# on stdout — "does match certificate" / "does NOT match certificate" — and
# the exit code only says whether it could read the file. Written as
# `if openssl … >/dev/null; then` (the obvious way, and the way this was
# written first) the test is vacuous: it accepts the first lineage on the
# box and throws away the one line that held the answer. Measured on
# OpenSSL 3.0.13; `-checkhost` is used rather than a hand-rolled SAN scrape
# because it does wildcard matching correctly and this box may hold a
# `*.elopros.com` cert.
covers() {  # $1 = name, $2 = a fullchain.pem
  local verdict
  verdict=$(openssl x509 -noout -checkhost "$1" -in "$2" 2>/dev/null) || return 1
  case "$verdict" in
    *"does match certificate"*) return 0 ;;   # "does NOT match" cannot match this
    *) return 1 ;;
  esac
}

# The two named candidates set the PREFERENCE order; the glob is the
# catch-all, because certbot renames a lineage `<name>-0001` when it has to
# and a hook that only knew two literal paths would refuse on a box that is
# perfectly healthy. A glob that matches nothing stays literal and fails the
# `-f` test, so the fallback costs nothing when it is not needed.
for cand in /etc/letsencrypt/live/"$SHARD_NAME" \
            /etc/letsencrypt/live/game.moreright.xyz \
            /etc/letsencrypt/live/*; do
  [ -f "$cand/fullchain.pem" ] || continue
  if covers "$SHARD_NAME" "$cand/fullchain.pem"; then
    LIVE=$cand
    break
  fi
done

# **Refuse rather than install a chain that cannot answer for the published
# name.** This is the check that was missing, and its absence cost a dark
# shard: on 2026-08-20 the served shard list moved to `game.elopros.com`
# while this box kept serving a chain whose only DnsName was
# `game.moreright.xyz`, so every joiner got `invalid peer certificate` and
# the client was right to refuse it (`tls_posture.rs` validates against the
# platform root store with no pin). Nothing on the box was in a failed
# state; the certificate was simply for a different name.
#
# Nonzero and loud: a renewal hook that quietly did nothing is what let the
# first one through. Note the check runs BEFORE the install, so a wrong
# chain never overwrites the copies a running shard is already serving.
if [ -z "$LIVE" ]; then
  echo "[copy-gates-cert] REFUSING: no lineage under /etc/letsencrypt/live" \
       "covers $SHARD_NAME — the shard would serve a certificate for a name" \
       "nobody dials. Fix: certbot certonly --expand -d game.moreright.xyz" \
       "-d $SHARD_NAME" >&2
  exit 1
fi

# Certbot sets RENEWED_LINEAGE per renewed cert; when it names a lineage that
# is not ours, do nothing. Unset (a manual run of the directory) falls
# through and copies, which is safe and is how you test this by hand.
case "${RENEWED_LINEAGE:-$LIVE}" in
  "$LIVE"|"$LIVE"/) ;;
  *) [ -n "${RENEWED_LINEAGE:-}" ] && exit 0 ;;
esac

install -d -o master -g master -m 755 "$DEST"
install -o master -g master -m 644 "$LIVE/fullchain.pem" "$DEST/fullchain.pem"
install -o master -g master -m 600 "$LIVE/privkey.pem"   "$DEST/privkey.pem"

# Restart only if the unit is actually installed and running. `restart` sends
# SIGTERM and waits, which is the shard's save path (`ops/gates-shard.service`
# — the KillSignal block); a certbot renewal must not be the thing that
# costs a hundred players their inventory.
if systemctl list-unit-files gates-shard.service >/dev/null 2>&1 \
   && systemctl is-active --quiet gates-shard.service; then
  systemctl restart gates-shard.service
  echo "[copy-gates-cert] refreshed $DEST from $LIVE and restarted gates-shard"
else
  echo "[copy-gates-cert] refreshed $DEST from $LIVE (gates-shard not running)"
fi
