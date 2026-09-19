# A second player can finish the wounded loop

Aim at an awake wounded body within the proposed 3 m hand reach and hold E
for six stationary seconds. The target owns one counter; helpers cannot
pool progress or inherit one another's elapsed time. The wound deadline
moves by each held tick, including time from an interrupted attempt, so a
hold can span the original roll deadline. Recovery keeps existing hp and
starts the existing re-wound cooldown.

The sim checks aim, terrain/structure cover, grounded movement and fresh
input before and after the tick. Releasing, moving, turning away, losing
cover or taking damage interrupts progress. A damaging debit also ends the
gesture when healing hides its net hp loss; the helper presses E again.
The client calls the same quantized cylinder/ray calculation to offer the
prompt. Server progress drives the helping readout and pauses the wounded
player's displayed countdown. The renderer does not decide the outcome.

Wire v66 adds a reliable target action, a fresh input bit and an absolute
progress event. Only the participants see progress. A per-client successful
send shadow retries changed state, including a dropped cancellation;
counting arrivals would have overrun or stalled under event backpressure.
Recovery remains the existing own fact. All three sim hold fields enter
the state hash and are cleared on save/restore; the save format is unchanged.

Tests cover exact duration, the old roll deadline, interruption and reset,
helper handoff, movement/decay, out-of-range/sleep/self refusal, a real wall,
a finishing-tick hit (also with simultaneous healing), save normalization,
and client picking. Ignoring the cover result deliberately fails the wall
test; restoring it passes. The server test wounds through combat and drives real
input/action/event codecs, including dropped progress and cancellation.
The shared parity probe includes a released hold then a completed recovery,
with a nonzero completion count; the allocation gate runs its tick/hash
portion inside the counted window, after startup construction.

All 110 wire fixtures are regenerated for v66. Their dispatcher now checks
the previously omitted loose-item sync fixture as well as both new fixtures.
The action-code spare count intentionally falls from eleven to ten. Replay
goldens change because the hashed Player state gains three fields, on top
of the road grading in the preceding commit. Together they make World
58 kB; the shadow-stack note follows that measured size. No
balance item, syringe, bandage target or medkit rule is introduced.
