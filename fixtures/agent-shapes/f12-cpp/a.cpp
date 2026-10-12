struct Box {
    template <typename T> T genM(T x) { return x; }
    template <typename T> static T genS(T x) { return x; }
    int ctlM(int x) { return x; }
    static int ctlS(int x) { return x; }
};
template <typename T> T genF(T x) { return x; }
int ctlF(int x) { return x; }
namespace ns { template <typename T> T genN(T x) { return x; } }
template <typename B> void useDep(B b) { b.template genM<int>(1); }
void useGen() {
    Box b;
    genF<int>(1);
    b.genM<int>(2);
    Box::genS<int>(3);
    ns::genN<int>(4);
}
void useCtl() {
    Box b;
    ctlF(1);
    b.ctlM(2);
    Box::ctlS(3);
}
