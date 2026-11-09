#!/usr/bin/env python3
"""Summarize `soak` JSON lines without treating missing clients as zero bytes."""
import json
import math
import sys


def percentile(values, percent):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * percent / 100) - 1)]


def summarize(lines):
    rows = [json.loads(line) for line in lines if line.startswith('{')]
    starts = [r for r in rows if r.get('kind') == 'start']
    jitters = [r for r in rows if r.get('kind') == 'jitter']
    ends = [r for r in rows if r.get('kind') == 'complete']
    clients = [r for r in rows if r.get('kind') == 'client']
    if len(starts) != 1 or len(jitters) != 1 or len(ends) != 1:
        raise ValueError('expected exactly one start, jitter and complete record')
    start, jitter, end = starts[0], jitters[0], ends[0]
    count = start['clients']
    if len(clients) != count or {r['client'] for r in clients} != set(range(count)):
        raise ValueError('missing or duplicate client reports')
    if (not end['passed'] or end['clients'] != count
            or jitter['minimum_clients_polled_100ms'] != count
            or jitter['client_ended_early']
            or jitter['elapsed_us'] < start['seconds'] * 1_000_000
            or any(r['elapsed_us'] < start['seconds'] * 1_000_000
                   or r['decode_errors'] or r['event_decode_errors'] or r['walk_truncated']
                   for r in clients)):
        raise ValueError('soak did not sustain the requested clients and duration')
    bins = jitter['absolute_error_us_log2_bins']
    if len(bins) != 32 or any(not isinstance(n, int) or n < 0 for n in bins):
        raise ValueError('invalid jitter histogram')
    total = sum(bins)
    if total == 0:
        raise ValueError('no tick interval samples')
    quantiles = {}
    for pct in [50, 90, 95, 99, 99.9]:
        rank, seen = math.ceil(total * pct / 100), 0
        for i, n in enumerate(bins):
            seen += n
            if seen >= rank:
                quantiles[str(pct)] = {'lower_us': 0 if i == 0 else 2 ** (i - 1),
                                      'upper_exclusive_us': None if i == len(bins) - 1 else 2 ** i}
                break
    bandwidth = {}
    for direction, fields in [('downstream', ('dg_in_bytes', 'stream_in_bytes')),
                               ('upstream', ('dg_out_bytes', 'stream_out_bytes'))]:
        values = [sum(r[f] for f in fields) for r in clients]
        rates = [v * 1_000_000 / r['elapsed_us'] for v, r in zip(values, clients)]
        bandwidth[direction] = {
            'total_bytes': sum(values), 'per_client_bytes_min': min(values),
            'per_client_bytes_median': percentile(values, 50),
            'per_client_bytes_p95': percentile(values, 95), 'per_client_bytes_max': max(values),
            'bytes_per_second_min': min(rates), 'bytes_per_second_median': percentile(rates, 50),
            'bytes_per_second_p95': percentile(rates, 95), 'bytes_per_second_max': max(rates),
        }
    return {'start': start, 'jitter': jitter, 'jitter_samples': total,
            'absolute_jitter_percentile_bounds': quantiles, 'bandwidth': bandwidth,
            'byte_scope': 'Application datagrams plus framed event/action streams. Excludes handshake, TLS/QUIC/UDP/IP headers and retransmissions. Byte counters are sampled across the common measurement window; each client reports its actual counter-sampling interval in elapsed_us. Ramp-up and drain are excluded.',
            'clients': clients}


if __name__ == '__main__':
    with open(sys.argv[1], encoding='utf-8') as source:
        print(json.dumps(summarize(source), indent=2))
