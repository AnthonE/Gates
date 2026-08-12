#!/bin/bash
# Certbot deploy hook: refresh the copies of game.moreright.xyz's chain that
# the Gates shard reads, and restart it so the new one is served.
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

DOMAIN=game.moreright.xyz
LIVE=/etc/letsencrypt/live/$DOMAIN
DEST=/home/master/gates-certs

[ -d "$LIVE" ] || exit 0
# Certbot sets RENEWED_LINEAGE per renewed cert; when it names a lineage that
# is not ours, do nothing. Unset (a manual run of the directory) falls
# through and copies, which is safe and is how you test this by hand.
case "${RENEWED_LINEAGE:-$LIVE}" in
  *"$DOMAIN"*) ;;
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
  echo "[copy-gates-cert] refreshed $DEST and restarted gates-shard"
else
  echo "[copy-gates-cert] refreshed $DEST (gates-shard not running)"
fi
