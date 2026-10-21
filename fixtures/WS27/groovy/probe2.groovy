class Box {
    Box(Closure init) { init() }
}
def top(cb, int n) { cb(n) }
items.collect { a, b -> a(b) }
items.inject(0) { String acc, int v -> acc(v) }
items.each { -> it() }
def (String u, int w) = f()
final z = 1
int r1 = 1, r2 = 2
for (def k in ks) { k() }
for (Map.Entry en : m) { en() }
try { go() } catch (e) { e() }
task hello { doLast { println it } }
def lam = (a2, b2) -> a2(b2)
