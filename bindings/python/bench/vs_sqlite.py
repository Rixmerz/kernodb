"""Kerno vs SQLite on the same workloads, same durability (fsync per commit)."""
import kerno, sqlite3, tempfile, time, os, json, random
from pathlib import Path
N=int(os.environ.get("N","5000")); P=(b"\xff" if os.environ.get("BIN") else b"")+b"x"*200
def du(p): return sum(f.stat().st_size for f in Path(p).rglob("*") if f.is_file())
def t(f):
    s=time.perf_counter(); r=f(); return r, time.perf_counter()-s
rows=[]
def row(name, k, q, unit="s"): rows.append((name,k,q)); print(f"{name:42s} kerno {k:9.3f}  sqlite {q:9.3f}  ratio {k/q if q else float('inf'):6.1f}x", flush=True)

kd=Path(tempfile.mkdtemp(dir="/tmp/claude-0"))/"k"; sd=Path(tempfile.mkdtemp(dir="/tmp/claude-0"))/"s.db"
db=kerno.open(kd)
sq=sqlite3.connect(sd, isolation_level=None); sq.execute("PRAGMA journal_mode=WAL"); sq.execute("PRAGMA synchronous=FULL")
sq.execute("create table unit(id integer primary key, type text, payload blob, rev int)")
sq.execute("create table rev(unit int, rev int, payload blob, primary key(unit,rev))")
sq.execute("create table rel(id integer primary key, a int, b int, type text)"); sq.execute("create index rel_a on rel(a,type)"); sq.execute("create index unit_type on unit(type)")
def sq_store(p):
    cur=sq.execute("insert into unit(type,payload,rev) values('s',?,1)",(p,)); sq.execute("insert into rev values(?,1,?)",(cur.lastrowid,p)); return cur.lastrowid

# 1 single commits
n=min(N,2000)
_,k=t(lambda:[db.store("s",P) for _ in range(n)])
def f():
    for _ in range(n):
        sq.execute("begin"); sq_store(P); sq.execute("commit")
_,q=t(f); row(f"store x{n}, one commit each (s)",k,q)
# 2 batch
_,k=t(lambda:db.commit([("store","s",P)]*N))
def f():
    sq.execute("begin"); [sq_store(P) for _ in range(N)]; sq.execute("commit")
_,q=t(f); row(f"store x{N}, one batch (s)",k,q)
kids=[u.id for u in db.query("s")]; sids=[r[0] for r in sq.execute("select id from unit")]
# 3 point reads
sample=random.sample(range(len(kids)),2000)
_,k=t(lambda:[db.read(kids[i]) for i in sample])
_,q=t(lambda:[sq.execute("select * from unit where id=?",(sids[i],)).fetchone() for i in sample]); row("point read x2000 (s)",k,q)
# 4 scan by type
_,k=t(lambda:db.query("s")); _,q=t(lambda:sq.execute("select * from unit where type='s'").fetchall()); row(f"scan all {len(kids)} of a type (s)",k,q)
# 5 updates with history, batch
ks=kids[:N]; ss=sids[:N]
_,k=t(lambda:db.commit([("update",i,P+b"2") for i in ks]))
def f():
    sq.execute("begin")
    for i in ss:
        sq.execute("update unit set payload=?, rev=rev+1 where id=?",(P+b"2",i)); sq.execute("insert into rev values(?,2,?)",(i,P+b"2"))
    sq.execute("commit")
_,q=t(f); row(f"update x{N} keeping history, one batch (s)",k,q)
# 6 relations batch
pairs=[(random.randrange(len(ks)),random.randrange(len(ks))) for _ in range(N)]
_,k=t(lambda:db.commit([("relate",ks[a],ks[b],"r") for a,b in pairs]))
def f():
    sq.execute("begin"); sq.executemany("insert into rel(a,b,type) values(?,?,'r')",[(ss[a],ss[b]) for a,b in pairs]); sq.execute("commit")
_,q=t(f); row(f"relate x{N}, one batch (s)",k,q)
# 7 neighbours
_,k=t(lambda:[db.related(ks[i]) for i in range(1000)])
_,q=t(lambda:[sq.execute("select * from rel where a=? ",(ss[i],)).fetchall() for i in range(1000)]); row("related() x1000 (s)",k,q)
_,k=t(lambda:db.relations("r")); _,q=t(lambda:sq.execute("select * from rel where type='r'").fetchall()); row("all relations of a type (s)",k,q)
# 8 history read
_,k=t(lambda:[db.history(ks[i]) for i in range(1000)])
_,q=t(lambda:[sq.execute("select * from rev where unit=? order by rev desc",(ss[i],)).fetchall() for i in range(1000)]); row("history x1000 (s)",k,q)
db.close(); sq.close()
print(f"disk: kerno {du(kd)/2**20:.1f} MB  sqlite {sum(f.stat().st_size for f in sd.parent.glob('s.db*'))/2**20:.1f} MB  ({len(kids)} units, revisions x2 for {N}, {N} relations)")
kopen=t(lambda:kerno.open(kd))[1]; print(f"open: kerno {kopen*1000:.0f} ms")
