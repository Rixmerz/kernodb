"""Kerno against SQLite from Python: the same workloads, the same data, an fsync per commit.

    python vs_sqlite.py --n 10000 --reps 3 --out results.json

Every repetition is a fresh process on fresh directories (ADR-0012); inside it the two engines
alternate workload by workload. The report is the median over the repetitions with its range.
`--data` must be on a real disk: on tmpfs the fsync both engines pay per commit is free.

Workloads: writes (one commit each, batches, updates that keep history, relations), point reads,
a scan by type, relation traversal, history, substring search and keyword (full-text) search.
SQLite gets its own indexes for the two searches: FTS5 `trigram` and FTS5 `unicode61`. The
substring search is also compared with SQLite's plain `LIKE '%needle%'` scan.

On a kerno build without `search_substring` the substring search is what a caller had to do
then: `query(type)` and filter in Python. The row says which of the two ran.
"""

import argparse
import json
import os
import random
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path

BATCH = 10_000  # ops per commit in the batch workloads: kerno's default limit, used for both
SEED = 20261122

SYLLABLES = "ba be bi bo bu ca ce ci co cu da de di do du fa fe fi fo fu ga ge gi go gu la le li lo lu ma me mi mo mu na ne ni no nu pa pe pi po pu ra re ri ro ru sa se si so su ta te ti to tu va ve vi vo vu".split()


def vocabulary(rng, size):
    words = set()
    while len(words) < size:
        words.add("".join(rng.choice(SYLLABLES) for _ in range(rng.randint(2, 4))))
    return sorted(words)


def make_data(n):
    """`n` symbol-like names (60 to 90 bytes) and `n` prose documents (about 200 bytes)."""
    rng = random.Random(SEED)
    words = vocabulary(rng, 3000)
    # A skewed pick, like real text: a few words are in many documents, most are rare.
    pick = lambda: words[min(int(rng.expovariate(1 / 300)), len(words) - 1)]
    syms, docs = [], []
    for i in range(n):
        cls = pick().capitalize() + pick().capitalize()
        # No `_` or `%` in the data: they are wildcards in LIKE, and a needle is a slice of a name.
        syms.append(f"{pick()}.{pick()}.{cls}.{pick()}{pick().capitalize()}({pick()}, {pick()}) -> {pick().capitalize()} #{i}")
        docs.append(" ".join(pick() for _ in range(rng.randint(22, 34))))
    return words, syms, docs


def make_queries(words, syms):
    rng = random.Random(SEED + 1)
    needles = []
    for _ in range(10):  # selective: a slice out of a real name, typed in lower case
        s = rng.choice(syms).split(" #")[0]
        start = rng.randrange(0, len(s) - 12)
        needles.append(s[start : start + rng.randint(7, 12)].lower())
    needles += [rng.choice(words)[:4] for _ in range(5)]  # common fragments, many hits
    needles += ["zzqx", "kernonone", "qqqqqq"]  # no hit
    needles += ["ba", "-> "]  # shorter than a trigram, and one with punctuation
    rare = [rng.choice(words[1500:]) for _ in range(10)]
    common = [rng.choice(words[:200]) for _ in range(5)]
    pairs = [f"{rng.choice(words[:500])} {rng.choice(words[1000:])}" for _ in range(5)]
    return needles, rare + common + pairs


def timed(f):
    start = time.perf_counter()
    out = f()
    return out, time.perf_counter() - start


def chunks(items):
    return (items[i : i + BATCH] for i in range(0, len(items), BATCH))


def du(path):
    return sum(f.stat().st_size for f in Path(path).rglob("*") if f.is_file())


def run_once(n, data_dir):
    import kerno

    words, syms, docs = make_data(n)
    needles, text_queries = make_queries(words, syms)
    rng = random.Random(SEED + 2)
    rows = {}

    def row(name, k, q, note=""):
        rows[name] = {"kerno": k, "sqlite": q, "note": note}
        print(f"{name:46s} kerno {k:10.4f}  sqlite {q:10.4f}  x{k / q if q else float('inf'):8.2f}  {note}", file=sys.stderr, flush=True)

    kdir, sfile = Path(data_dir) / "kerno", Path(data_dir) / "sqlite" / "s.db"
    sfile.parent.mkdir(parents=True)
    db = kerno.open(kdir)
    sq = sqlite3.connect(sfile, isolation_level=None)
    sq.execute("PRAGMA journal_mode=WAL")
    sq.execute("PRAGMA synchronous=FULL")
    sq.executescript(
        """
        create table unit(id integer primary key, type text, payload text, rev int);
        create index unit_type on unit(type);
        create table rev(unit int, rev int, payload text, primary key(unit, rev));
        create table rel(id integer primary key, a int, b int, type text);
        create index rel_a on rel(a, type);
        create table sym(id integer primary key, body text);
        create virtual table sym_tri using fts5(body, tokenize='trigram');
        create table doc(id integer primary key, body text);
        create virtual table doc_fts using fts5(body);
        """
    )

    def sq_store(p):
        cur = sq.execute("insert into unit(type, payload, rev) values('s', ?, 1)", (p,))
        sq.execute("insert into rev values(?, 1, ?)", (cur.lastrowid, p))

    def sq_batches(items, each):
        for part in chunks(items):
            sq.execute("begin")
            for item in part:
                each(item)
            sq.execute("commit")

    # ---- writes
    single = docs[: min(n, 2000)]
    _, k = timed(lambda: [db.store("s", p) for p in single])
    _, q = timed(lambda: [(sq.execute("begin"), sq_store(p), sq.execute("commit")) for p in single])
    row("store x2000, one commit each", k, q)

    _, k = timed(lambda: [db.commit([("store", "s", p) for p in part]) for part in chunks(docs)])
    _, q = timed(lambda: sq_batches(docs, sq_store))
    row(f"store x{n}, batches of {BATCH}", k, q)

    kids = [u.id for u in db.query("s")]
    sids = [r[0] for r in sq.execute("select id from unit")]

    # ---- point reads and a scan by type
    sample = [rng.randrange(len(kids)) for _ in range(2000)]
    _, k = timed(lambda: [db.read(kids[i]) for i in sample])
    _, q = timed(lambda: [sq.execute("select * from unit where id=?", (sids[i],)).fetchone() for i in sample])
    row("point read x2000", k, q)

    _, k = timed(lambda: db.query("s"))
    _, q = timed(lambda: sq.execute("select * from unit where type='s'").fetchall())
    row(f"scan all {len(kids)} units of a type", k, q)

    # ---- updates that keep history
    ks, ss = kids[:n], sids[:n]
    new = [d + " v2" for d in docs]
    _, k = timed(lambda: [db.commit([("update", i, p) for i, p in part]) for part in chunks(list(zip(ks, new)))])

    def sq_update(item):
        i, p = item
        sq.execute("update unit set payload=?, rev=rev+1 where id=?", (p, i))
        sq.execute("insert into rev values(?, 2, ?)", (i, p))

    _, q = timed(lambda: sq_batches(list(zip(ss, new)), sq_update))
    row(f"update x{n} keeping history, batches", k, q)

    # ---- relations
    pairs = [(rng.randrange(n), rng.randrange(n)) for _ in range(n)]
    _, k = timed(lambda: [db.commit([("relate", ks[a], ks[b], "r") for a, b in part]) for part in chunks(pairs)])
    _, q = timed(lambda: sq_batches(pairs, lambda p: sq.execute("insert into rel(a, b, type) values(?, ?, 'r')", (ss[p[0]], ss[p[1]]))))
    row(f"relate x{n}, batches", k, q)

    _, k = timed(lambda: [db.related(ks[i]) for i in range(1000)])
    _, q = timed(lambda: [sq.execute("select * from rel where a=?", (ss[i],)).fetchall() for i in range(1000)])
    row("related() x1000", k, q)

    def kerno_two_hops():
        return [[db.related(r.to_id) for r in db.related(ks[i])] for i in range(200)]

    def sq_two_hops():
        return [sq.execute("select r2.* from rel r1 join rel r2 on r2.a = r1.b where r1.a=?", (ss[i],)).fetchall() for i in range(200)]

    _, k = timed(kerno_two_hops)
    _, q = timed(sq_two_hops)
    row("two-hop traversal x200", k, q)

    _, k = timed(lambda: db.relations("r"))
    _, q = timed(lambda: sq.execute("select * from rel where type='r'").fetchall())
    row("all relations of a type", k, q)

    _, k = timed(lambda: [db.history(ks[i]) for i in range(1000)])
    _, q = timed(lambda: [sq.execute("select * from rev where unit=? order by rev desc", (ss[i],)).fetchall() for i in range(1000)])
    row("history x1000", k, q)

    # ---- substring search
    indexed = hasattr(db, "search_substring")
    if indexed:
        db.set_substring("sym", True)
    _, k = timed(lambda: [db.commit([("store", "sym", p) for p in part]) for part in chunks(syms)])

    def sq_sym(p):
        cur = sq.execute("insert into sym(body) values(?)", (p,))
        sq.execute("insert into sym_tri(rowid, body) values(?, ?)", (cur.lastrowid, p))

    _, q = timed(lambda: sq_batches(syms, sq_sym))
    row(f"store x{n} substring-indexed, batches", k, q, "kerno trigram index" if indexed else "kerno has no substring index")

    if indexed:
        kerno_find = lambda needle: db.search_substring(needle, types=["sym"])
    else:
        kerno_find = lambda needle: [u.id for u in db.query("sym") if needle in u.payload.decode().lower()]
    khits, k = timed(lambda: [len(kerno_find(x)) for x in needles])
    qhits, q = timed(lambda: [len(sq.execute("select id from sym where body like ?", (f"%{x}%",)).fetchall()) for x in needles])
    thits, t = timed(lambda: [len(sq.execute("select rowid from sym_tri where body like ?", (f"%{x}%",)).fetchall()) for x in needles])
    assert khits == qhits == thits, (khits, qhits, thits)
    how = "kerno index" if indexed else "kerno scan in Python"
    row(f"substring search x{len(needles)} vs LIKE scan", k, q, how)
    row(f"substring search x{len(needles)} vs FTS5 trigram", k, t, how)
    rows["substring hits"] = {"kerno": sum(khits), "sqlite": sum(qhits), "note": "matches over all needles"}

    # ---- keyword search
    db.set_fulltext("doc", True)
    _, k = timed(lambda: [db.commit([("store", "doc", p) for p in part]) for part in chunks(docs)])

    def sq_doc(p):
        cur = sq.execute("insert into doc(body) values(?)", (p,))
        sq.execute("insert into doc_fts(rowid, body) values(?, ?)", (cur.lastrowid, p))

    _, q = timed(lambda: sq_batches(docs, sq_doc))
    row(f"store x{n} keyword-indexed, batches", k, q)

    fts = lambda text: " OR ".join(text.split())
    rounds = 5
    khits, k = timed(lambda: [[len(db.search_text(x, limit=10, types=["doc"])) for x in text_queries] for _ in range(rounds)])
    qhits, q = timed(lambda: [[len(sq.execute("select rowid from doc_fts where doc_fts match ? order by rank limit 10", (fts(x),)).fetchall()) for x in text_queries] for _ in range(rounds)])
    assert khits == qhits, (khits, qhits)
    row(f"keyword search x{len(text_queries) * rounds}, top 10", k, q)
    # Both engines must find the same documents, not only the same number in the top 10.
    for x in text_queries[:5]:
        found = len(db.search_text(x, limit=10 * n, types=["doc"]))
        assert found == sq.execute("select count(*) from doc_fts where doc_fts match ?", (fts(x),)).fetchone()[0], x

    _, k = timed(db.close)
    _, q = timed(sq.close)
    row("close", k, q)
    rows["disk MiB"] = {"kerno": du(kdir) / 2**20, "sqlite": du(sfile.parent) / 2**20, "note": "after close"}
    _, k = timed(lambda: kerno.open(kdir).close())
    _, q = timed(lambda: sqlite3.connect(sfile).execute("select count(*) from unit").fetchone())
    row("reopen", k, q)
    return rows


def git_commit():
    try:
        return subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, cwd=Path(__file__).parent).stdout.strip()
    except OSError:
        return ""


def fmt(x):
    return f"{x:.3g}" if x < 100 else f"{x:.0f}"


def report(runs):
    print("| Workload | Kerno | SQLite | Kerno / SQLite | |")
    print("|---|---|---|---|---|")
    for name in runs[0]:
        k = [r[name]["kerno"] for r in runs]
        q = [r[name]["sqlite"] for r in runs]
        cell = lambda v: f"{fmt(statistics.median(v))} [{fmt(min(v))}-{fmt(max(v))}]"
        ratio = statistics.median(k) / statistics.median(q) if statistics.median(q) else float("inf")
        print(f"| {name} | {cell(k)} | {cell(q)} | {ratio:.2f} | {runs[0][name]['note']} |")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--n", type=int, default=10_000, help="units per workload")
    ap.add_argument("--reps", type=int, default=3, help="repetitions, each a fresh process")
    ap.add_argument("--data", default="bench-data", help="scratch directory, on the disk to measure")
    ap.add_argument("--out", help="write the raw repetitions and the environment here, as JSON")
    ap.add_argument("--once", metavar="DIR", help=argparse.SUPPRESS)
    args = ap.parse_args()
    if args.once:
        print(json.dumps(run_once(args.n, args.once)))
        return
    Path(args.data).mkdir(parents=True, exist_ok=True)
    runs, load = [], []
    for rep in range(args.reps):
        load.append(os.getloadavg()[0])
        with tempfile.TemporaryDirectory(dir=args.data) as scratch:
            print(f"-- repetition {rep + 1}/{args.reps}, n={args.n}", file=sys.stderr, flush=True)
            out = subprocess.run([sys.executable, __file__, "--n", str(args.n), "--once", scratch], stdout=subprocess.PIPE, check=True)
            runs.append(json.loads(out.stdout))
    report(runs)
    if args.out:
        import kerno

        env = {
            "n": args.n,
            "reps": args.reps,
            "commit": git_commit(),
            "kerno": getattr(kerno, "__version__", ""),
            "sqlite": sqlite3.sqlite_version,
            "python": sys.version.split()[0],
            "loadavg_1m_at_each_start": load,
            "unit": "seconds, except the rows named hits and MiB",
        }
        Path(args.out).write_text(json.dumps({"env": env, "runs": runs}, indent=1) + "\n")


if __name__ == "__main__":
    main()
