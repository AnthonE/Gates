# World resolution with a full-resolution HUD

Render scale is a saved graphics row, 50–100% in five-point steps. Every
preset remains at 100%. The percentage applies to each physical dimension,
so 50% shades one quarter of the world pixels. Invalid saved values fall
back to the preset; valid integers outside the range clamp to its ends.

Below 100%, the eye renders its existing post effects into an sRGB target.
A geometry-free presentation camera composes that image and the UI at the
window's physical resolution. Its material samples RGB and writes alpha one:
native atmosphere preserves the skybox's zero alpha between clouds, so an
ordinary ImageNode made holes in an otherwise finished sky. A later native
headland image exposed this after the first smoke; browser sky texels are
opaque and could not expose it. The presentation owns opacity, not the sky. Resizing reuses the image handle; returning
to 100% restores the original window target and releases the presentation
entities. Leaving the world releases them too. Settled frames do not mark
the image changed. This adds an image and a composition pass; it reduces
pixel work, not world generation, streaming, simulation or draw submission.

The arithmetic tests cover high DPI, tiny extents, saved-scale joins,
resize, handle reuse, the default direct path, and restoration to 100%.
Settings tests cover round trips, clamping and malformed values. Omitting
the eye-target switch deliberately fails the join test; restoring it passes.

Manual checks used a local shard, seed 20260731. Native: High at 50%,
1280×720 under Xvfb/lavapipe, spawn (1558, 644), with the world loaded.
Browser: Chromium/SwiftShader WebGL2, spawn (1500, 600), 640×360 then 960×720;
100% to 50%, SMAA and bloom enabled together, resize while scaled, and back
to 100%. The world remained visible and the HUD stayed sharp. No fatal page
errors or failed game requests were reported. Temporary screenshots and
logs are outside the repository. These checks establish that the paths
run; they do not measure hardware frame time or replace appearance review.
