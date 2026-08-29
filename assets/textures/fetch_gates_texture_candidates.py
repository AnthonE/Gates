#!/usr/bin/env python3
"""Acquire pristine Gates texture candidates; do not score them."""
from __future__ import annotations
import argparse, csv, hashlib, html, json, re, sys, time
from pathlib import Path
from urllib.parse import parse_qs, quote, unquote, urljoin, urlparse
from urllib.request import Request, urlopen

UA="GatesTextureCandidateFetcher/1.0 (+https://github.com/AnthonE/Gates)"
SCORE=("diff","albedo","basecolor","base_color","base color","alpha","opacity")
PBR=SCORE+("rough","normal","nor_gl","nor_dx","ao","ambient_occlusion","mask")

def get(url, binary=True, referer=None):
    h={"User-Agent":UA,"Accept":"*/*"}
    if referer: h["Referer"]=referer
    req=Request(url, headers=h)
    with urlopen(req, timeout=90) as r:
        data=r.read()
    return data if binary else data.decode("utf-8","replace")

def safe(s):
    s=re.sub(r"[^A-Za-z0-9._-]+","_",s.strip())
    return s.strip("._") or "asset"

def poly_urls(asset_id,res,maps):
    data=json.loads(get("https://api.polyhaven.com/files/"+quote(asset_id),False))
    found=[]
    def walk(node,path=()):
        if isinstance(node,dict):
            if isinstance(node.get("url"),str): found.append((path,node["url"]))
            for k,v in node.items(): walk(v,path+(str(k),))
        elif isinstance(node,list):
            for i,v in enumerate(node): walk(v,path+(str(i),))
    walk(data)
    terms=PBR if maps=="pbr" else SCORE
    out=[]; seen=set()
    for path,url in found:
        p=" ".join(path).lower(); u=url.lower()
        if res.lower() not in p and res.lower() not in u: continue
        if not any(t in p or t in u for t in terms): continue
        if any(x in u for x in (".blend",".fbx",".gltf",".glb",".hdr",".exr")): continue
        if url not in seen: seen.add(url); out.append((path,url))
    return out

def oga_urls(page,hint):
    txt=get(page,False)
    hrefs=re.findall("href=[\\\"']([^\\\"']+)[\\\"']",txt,flags=re.I)
    urls=[]
    for h in hrefs:
        u=urljoin(page,html.unescape(h))
        if "/sites/default/files/" in u: urls.append(u)
    urls=list(dict.fromkeys(urls))
    if hint:
        out=[]
        for part in hint.split("|"):
            needle=part.lower().replace(" ","")
            if not needle: continue
            m=[u for u in urls if needle in unquote(u).lower().replace(" ","")]
            if not m: raise RuntimeError("file hint not found on page: "+part)
            if m[0] not in out: out.append(m[0])
        return out
    exts=(".png",".jpg",".jpeg",".zip",".7z")
    return [u for u in urls if unquote(urlparse(u).path).lower().endswith(exts)][:8]

CGB_CDN="https://cgbookcase-volume.b-cdn.net/t/"
CGB_REF="https://www.cgbookcase.com/"

def cgb_url(page,res):
    """cgbookcase's button is client-side: the page carries the archive name, the
    volume CDN serves it only with a cgbookcase.com Referer."""
    txt=get(page,False)
    m=re.search(r"/textures/thanks\?t=([^\"'&]+)",txt)
    if not m: raise RuntimeError("cgbookcase download name not found on page")
    fname=unquote(html.unescape(m.group(1)))
    avail=[int(x) for x in re.findall(r"<option value=\"(\d+)\"",txt)]
    want=int(res.upper().rstrip("K"))
    if avail and want not in avail:
        want=max(avail); print(f"  note: {res} unavailable, using {want}K (offered: {avail})")
    fname=re.sub(r"_\d+K\.zip$",f"_{want}K.zip",fname)
    return CGB_CDN+quote(fname), fname

def wm_url(title):
    api=("https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo&iiprop=url&titles="+quote(title))
    data=json.loads(get(api,False))
    for p in data.get("query",{}).get("pages",{}).values():
        info=p.get("imageinfo") or []
        if info and info[0].get("url"): return info[0]["url"]
    raise RuntimeError("Wikimedia original URL not resolved")

def main():
    ap=argparse.ArgumentParser()
    ap.add_argument("--csv",default=str(Path(__file__).with_name("gates_texture_candidates.csv")))
    ap.add_argument("--dest",default="assets/textures/candidates")
    ap.add_argument("--sets",default="9.5,9.6,9.7,9.8,9.9,9.10")
    ap.add_argument("--tier",default="A,B")
    ap.add_argument("--resolution",default="2K",choices=["1K","2K","4K","8K"])
    ap.add_argument("--maps",default="score",choices=["score","pbr"])
    ap.add_argument("--decision",default="")
    ap.add_argument("--dry-run",action="store_true")
    a=ap.parse_args()
    sets={x.strip() for x in a.sets.split(",") if x.strip()}
    tiers={x.strip().upper() for x in a.tier.split(",") if x.strip()}
    with open(a.csv,newline="",encoding="utf-8") as f: data=list(csv.DictReader(f))
    data=[r for r in data if r["Set"] in sets and r["Acquisition tier"].upper() in tiers
          and (not a.decision or r["Decision"]==a.decision)]
    root=Path(a.dest); log=[]; manual=[]
    print(f"{len(data)} candidate rows selected")
    for i,r in enumerate(data,1):
        cid=r["Candidate ID"]; mode=r["Fetch mode"]; print(f"[{i}/{len(data)}] {cid}")
        if mode=="manual_cgbookcase":  # legacy rows; cgbookcase_page fetches these now
            manual.append((cid,r["Source URL"])); log.append([cid,"manual","","","",r["Source URL"]]); continue
        try:
            if mode=="polyhaven_api":
                dls=[(u,"_".join(safe(x) for x in p[-4:] if x) or r["Provider ID"]) for p,u in poly_urls(r["Provider ID"],a.resolution,a.maps)]
                if not dls: raise RuntimeError("no matching Poly Haven image maps at requested resolution")
            elif mode=="ambientcg_zip":
                dls=[(f"https://ambientcg.com/get?file={quote(r['Provider ID'])}_{a.resolution.upper()}-PNG.zip",r["Provider ID"])]
            elif mode=="oga_page":
                dls=[(u,r["File hint"] or cid) for u in oga_urls(r["Source URL"],r["File hint"])]
                if not dls: raise RuntimeError("no downloadable OGA files resolved")
            elif mode=="cgbookcase_page":
                u,fname=cgb_url(r["Source URL"],a.resolution); dls=[(u,fname)]
            elif mode=="wikimedia_api":
                dls=[(wm_url(r["Provider ID"]),r["Provider ID"])]
            else: raise RuntimeError("unknown fetch mode "+mode)
            for n,(url,label) in enumerate(dls,1):
                if a.dry_run: print("  ->",url); log.append([cid,"dry-run","","","",url]); continue
                b=get(url,True,CGB_REF if CGB_CDN in url else None)
                name=unquote(urlparse(url).path.split("/")[-1]) or label
                if "." not in name:  # e.g. ambientCG's /get?file=<archive>
                    q=parse_qs(urlparse(url).query).get("file")
                    name=unquote(q[0]) if q else label
                name=safe(name); name=(f"{n:02d}_"+name) if len(dls)>1 else name
                path=root/r["Set"].replace(".","_")/cid/name; path.parent.mkdir(parents=True,exist_ok=True); path.write_bytes(b)
                h=hashlib.sha256(b).hexdigest()
                print(f"  saved {path} ({len(b):,} bytes; {h[:12]}…)")
                log.append([cid,"fetched",str(path),h,str(len(b)),url]); time.sleep(.15)
        except Exception as e:
            print("  ERROR:",e,file=sys.stderr); log.append([cid,"error","","","",r["Source URL"]+" :: "+str(e)])
    root.mkdir(parents=True,exist_ok=True)
    with (root/"_fetched.csv").open("w",newline="",encoding="utf-8") as f:
        w=csv.writer(f); w.writerow(["Candidate ID","Status","Path","SHA256","Bytes","Resolved URL / error"]); w.writerows(log)
    if manual:
        (root/"_manual_downloads.txt").write_text("\n".join(f"{c}\t{u}" for c,u in manual)+"\n",encoding="utf-8")
    print("wrote",root/"_fetched.csv")

if __name__=="__main__": main()
