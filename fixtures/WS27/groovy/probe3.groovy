class C {
    def run(cb, x = 1, def y) {
        def c1 = { String a, int b -> a(b) }
        def c2 = { a ->
            a()
        }
        items.each { Map<String, Integer> m, b ->
            m()
        }
        items.each { item ->
            item()
        }
        items.each {
            it.foo()
        }
        def (p, q) = pair()
        p()
        for (def k in ks) { k() }
        try {
            go()
        } catch (e) {
            e()
        }
        if (ok) { it() }
        switch (v) { case 1: def sw = 2; sw(); break }
        while (true) { def wl = 1 }
    }
}
