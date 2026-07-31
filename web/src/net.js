// The WebTransport session (NETCODE.md §2): one connection, datagrams for
// the hot flows, one bidi stream for the handshake. Client-side queue
// tuning per §2.1 — low-latency hint, 50 ms outgoing max age so stale
// inputs die in the queue, and every send clamped against the LIVE
// maxDatagramSize (the trap list: an oversize browser datagram "succeeds"
// and sends nothing).

/** "ab:cd:…" dotted hex (the shard log format) or plain hex → 32 bytes. */
export function parseCertHash(s) {
  const hex = (s || "").replace(/[^0-9a-fA-F]/g, "");
  if (hex.length === 0) return null;
  if (hex.length !== 64) throw new Error("cert hash must be 32 bytes of hex");
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

export async function connect(url, certHex) {
  if (!("WebTransport" in globalThis)) {
    throw new Error(
      "this browser has no WebTransport (need Chrome 97+, Edge 98+, Firefox 125+, Safari 26.4+)",
    );
  }
  const opts = { congestionControl: "low-latency" };
  const hash = parseCertHash(certHex);
  if (hash) {
    opts.serverCertificateHashes = [{ algorithm: "sha-256", value: hash }];
  }
  const wt = new WebTransport(url, opts);
  await wt.ready;
  try {
    wt.datagrams.outgoingMaxAge = 50; // NETCODE §2.1
  } catch {
    // Older engines expose it read-only; the default queue still drains.
  }
  return wt;
}

/** u16-LE length-prefixed frame on the bidi lane (server/src/net.rs). */
export async function handshake(wt, helloBytes) {
  const bidi = await wt.createBidirectionalStream();
  const writer = bidi.writable.getWriter();
  const frame = new Uint8Array(2 + helloBytes.length);
  frame[0] = helloBytes.length & 0xff;
  frame[1] = (helloBytes.length >> 8) & 0xff;
  frame.set(helloBytes, 2);
  await writer.write(frame);
  writer.releaseLock();

  const reader = bidi.readable.getReader();
  let buf = new Uint8Array(0);
  for (;;) {
    if (buf.length >= 2) {
      const len = buf[0] | (buf[1] << 8);
      if (len === 0 || len > 64) throw new Error("bad handshake frame");
      if (buf.length >= 2 + len) {
        reader.releaseLock();
        return buf.subarray(2, 2 + len);
      }
    }
    const { value, done } = await reader.read();
    if (done) throw new Error("stream closed during handshake");
    const next = new Uint8Array(buf.length + value.length);
    next.set(buf);
    next.set(value, buf.length);
    buf = next;
  }
}

/** Fire the datagram RX pump; onBytes is called per datagram. */
export function pumpDatagrams(wt, onBytes, onClosed) {
  (async () => {
    const reader = wt.datagrams.readable.getReader();
    try {
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        onBytes(value);
      }
    } catch {
      // Connection died; wt.closed below reports it.
    }
    onClosed();
  })();
}

/**
 * Datagram sender with a small rotating copy pool: the bytes leave wasm
 * memory before the async write settles, and steady state allocates
 * nothing but subarray views (DESIGN.md L8).
 */
export function makeSender(wt) {
  const writer = wt.datagrams.writable.getWriter();
  const pool = [
    new Uint8Array(1200),
    new Uint8Array(1200),
    new Uint8Array(1200),
    new Uint8Array(1200),
  ];
  let next = 0;
  const stats = { sent: 0, oversize: 0 };
  return {
    stats,
    send(src, len) {
      const max = wt.datagrams.maxDatagramSize; // live value, every send
      if (len > max || len > 1200) {
        stats.oversize += 1;
        return false;
      }
      const buf = pool[next];
      next = (next + 1) & 3;
      buf.set(src.subarray(0, len));
      writer.write(buf.subarray(0, len)).catch(() => {});
      stats.sent += 1;
      return true;
    },
  };
}
