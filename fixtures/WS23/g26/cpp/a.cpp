class Tool { public: void work() {} };
class Other { public: void work() {} };
Tool* make() { return new Tool(); }
static const Other& other() { static Other o; return o; }
void use() {
    auto t = make();
    t->work();
    make()->work();
    auto& o = other();
    o.work();
    other().work();
}
void a() { auto x = make(); x->work(); }
void b() { auto x = other(); x.work(); }
