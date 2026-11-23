import re, sys, collections

def demangle(name):
    # Rust v0 names (_RNv..., _RNvX...) hold length-prefixed identifiers; keep the last few.
    if not name.startswith("_R"):
        return name
    parts = re.findall(r"(\d+)([A-Za-z_][A-Za-z0-9_]*)", name)
    idents = [s for n, s in parts if int(n) == len(s)]
    return "rust::" + "::".join(idents[-4:]) if idents else name

def frame_name(f):
    f = f.split(" (")[0]
    return demangle(f)

self_c = collections.Counter()
incl = collections.Counter()
total = 0
for line in open(sys.argv[1]):
    line = line.rstrip()
    if not line:
        continue
    stack, _, cnt = line.rpartition(" ")
    if len(sys.argv) > 3 and sys.argv[3] not in stack:
        continue
    cnt = int(cnt)
    frames = [frame_name(f) for f in stack.split(";")]
    total += cnt
    self_c[frames[-1]] += cnt
    for f in set(frames):
        incl[f] += cnt
print("total samples", total)
print("== top self ==")
for f, c in self_c.most_common(int(sys.argv[2])):
    print(f"{100*c/total:6.2f}%  {f}")
print("== top inclusive ==")
for f, c in incl.most_common(int(sys.argv[2])):
    print(f"{100*c/total:6.2f}%  {f}")
