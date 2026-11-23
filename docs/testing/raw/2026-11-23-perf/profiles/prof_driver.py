"""Single-workload driver for py-spy: one process, one Kerno workload repeated."""
import sys, time, random
from pathlib import Path
sys.path.insert(0, "/home/rixmerz/Personal/Projects/kernodb/bindings/python/bench")
import vs_sqlite as vs
import kerno

wl, n, d, reps = sys.argv[1], int(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4])
words, syms, docs = vs.make_data(n)
rng = random.Random(vs.SEED + 2)
db = kerno.open(d / "k")

def store_all(kind, items):
    for part in vs.chunks(items):
        db.commit([("store", kind, p) for p in part])

def timed(label, f):
    t = time.perf_counter(); out = f(); print(f"{label} {time.perf_counter() - t:.4f}", file=sys.stderr, flush=True); return out

if wl == "batch-store":
    for r in range(reps):
        sub = kerno.open(d / f"b{r}")
        timed("batch-store", lambda: [sub.commit([("store", "s", p) for p in part]) for part in vs.chunks(docs)])
        sub.close()
elif wl == "update":
    store_all("s", docs); kids = [u.id for u in db.query("s")]; new = [x + " v2" for x in docs]
    for r in range(reps):
        timed("update", lambda: [db.commit([("update", i, p) for i, p in part]) for part in vs.chunks(list(zip(kids, new)))])
        new = [x + " v%d" % r for x in new]
elif wl == "relate":
    store_all("s", docs); kids = [u.id for u in db.query("s")]
    pairs = [(rng.randrange(n), rng.randrange(n)) for _ in range(n)]
    for r in range(reps):
        timed("relate", lambda: [db.commit([("relate", kids[a], kids[b], "r") for a, b in part]) for part in vs.chunks(pairs)])
elif wl == "traverse":
    store_all("s", docs); kids = [u.id for u in db.query("s")]
    pairs = [(rng.randrange(n), rng.randrange(n)) for _ in range(n)]
    for part in vs.chunks(pairs):
        db.commit([("relate", kids[a], kids[b], "r") for a, b in part])
    for r in range(reps):
        timed("related", lambda: [db.related(kids[i]) for i in range(1000)])
        timed("two-hop", lambda: [[db.related(x.to_id) for x in db.related(kids[i])] for i in range(200)])
elif wl == "scan":
    store_all("s", docs)
    for r in range(reps):
        timed("scan", lambda: db.query("s"))
elif wl == "point-read":
    store_all("s", docs); kids = [u.id for u in db.query("s")]; sample = [rng.randrange(len(kids)) for _ in range(2000)]
    for r in range(reps):
        timed("point-read", lambda: [db.read(kids[i]) for i in sample])
elif wl == "sym-store":
    db.set_substring("sym", True)
    for r in range(reps):
        sub = kerno.open(d / f"y{r}"); sub.set_substring("sym", True)
        timed("sym-store", lambda: [sub.commit([("store", "sym", p) for p in part]) for part in vs.chunks(syms)])
        sub.close()
elif wl == "doc-store":
    for r in range(reps):
        sub = kerno.open(d / f"f{r}"); sub.set_fulltext("doc", True)
        timed("doc-store", lambda: [sub.commit([("store", "doc", p) for p in part]) for part in vs.chunks(docs)])
        sub.close()
elif wl == "close":
    for r in range(reps):
        sub = kerno.open(d / f"c{r}")
        for part in vs.chunks(docs): sub.commit([("store", "s", p) for p in part])
        timed("close", sub.close)
elif wl == "reopen":
    store_all("s", docs[:n]); db.close()
    for r in range(reps):
        timed("reopen", lambda: kerno.open(d / "k").close())
else:
    raise SystemExit("unknown workload " + wl)
