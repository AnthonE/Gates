# Gates texture candidate bundle

This is an acquisition/measurement queue for the six foliage/bark sets in Gates.

- Total candidate rows: **84**
- CC0: **80**
- CC-BY: **4**
- cgbookcase rows: **7** (fetched automatically; see `cgbookcase_page` below)
- Per set: 9.5=14, 9.6=17, 9.7=14, 9.8=10, 9.9=14, 9.10=15

## Acquisition tiers are not art scores

- **A**: ready to measure — correct-ish identity and/or alpha/opacity supplied.
- **B**: pool expansion — archive extraction, generic morphology, or identity screening needed.
- **C**: legal fallback — masking and/or CC-BY attribution work required.

Gates' actual choice still comes from measurement.

## Fetch

The script, the CSV and the sheet live here beside `MANIFEST.md` (they were
written at the repo root and moved in when they were committed, 2026-08-14).
The script reads the CSV **beside itself**, so keep the three together; `--dest`
is repo-root-relative, so run it **from the repo root**:

```bash
python assets/textures/fetch_gates_texture_candidates.py --sets 9.5,9.6,9.7,9.8,9.9,9.10 --tier A,B
```

Defaults: **2K**, and score-stage maps (albedo/diffuse + alpha/opacity where exposed).
For finalists:

```bash
python assets/textures/fetch_gates_texture_candidates.py --tier A,B --maps pbr --decision finalist
```

Tier C rows (the CC-BY fallbacks) are not in that default; add `--tier A,B,C` to include them.

Downloads go to `assets/textures/candidates/` and `_fetched.csv` records resolved URL, SHA-256 and byte size.

### Fetch modes

| Mode | Notes |
| --- | --- |
| `polyhaven_api` | Poly Haven file API; model pages treated as texture containers only. |
| `ambientcg_zip` | `…/get?file=<ID>_<RES>-PNG.zip`; the archive name comes from the query, not the path. |
| `oga_page` | Scrapes `/sites/default/files/` links. `File hint` is a substring match; several hints can be `\|`-separated. OGA renames files on re-upload (`fern.zip` → `fern_0.zip`), so hints go stale — the error message names the hint that missed. |
| `wikimedia_api` | Resolves the original (not a thumbnail) via `imageinfo`. |
| `cgbookcase_page` | The download button is client-side: the page carries `…/thanks?t=<Name>_MR_<R>K.zip`, and `cgbookcase-volume.b-cdn.net` serves that name only with a `cgbookcase.com` Referer. The mode reads the archive name and the offered resolutions off the page, and falls back to the maximum offered if the requested resolution does not exist. |

## Acquisition status

All 84 rows fetched: 188 files, 1.55 GB, 0 errors. `_verify.csv` records a magic-byte check
per file (188/188 ok) — that check exists because a silently-saved HTML error page looks like a
texture until the estimator chokes on it.

Known gap: **96-CGB-GREENLEAF01** — cgbookcase's archive contains only `Opacity`, at both 1K and 2K.
There is no BaseColor from the publisher, so that row cannot be albedo-scored as fetched. The site's
thumbnail BaseColor is not a substitute; it is resampled and would contaminate the measurement.

## Archives are left packed

29 of the 188 files are `.zip`/`.7z` (ambientCG map sets, OGA packs, cgbookcase sets) totalling
~0.92 GiB unpacked. They are deliberately **not** extracted: extract a candidate when you are about
to measure it, into a directory beside the archive, and keep the archive as the pristine original.
`.7z` needs `p7zip`/`py7zr`; `.zip` needs nothing.

## Measure, then pack

The fetcher intentionally does **not** recreate Gates' estimator. `ART.md §7` says to use the
**shipped estimator itself** for gain span, albedo SD and directional anisotropy. Fill those columns
in the sheet/CSV after acquisition.

Recommended sequence:

1. Acquire pristine candidate.
2. Preserve the original.
3. If masking is required, preserve the derived cutout separately and record the step.
4. Run Gates' own estimator.
5. Record gain span, albedo SD, directional anisotropy and resulting keep.
6. Reject/finalist/select.
7. **Only then** pack winners into the runtime atlas.
8. Add selected source + license + measurements to `assets/textures/MANIFEST.md`.

Do not atlas first: transparent padding and unrelated species can contaminate a measurement that is
supposed to describe one source.

## Identity notes

Exact ready-made CC0 ash/aspen cutouts are scarce in the sources checked. The pool therefore includes
two CC-BY 2.0 species photographs (Fraxinus and Populus tremula) as fallbacks; both require masking and
a NOTICE entry only if actually selected.

Poly Haven model pages are treated strictly as **texture containers**: fetch leaf/twig/frond maps and
ignore the meshes, keeping Gates' generated geometry.
