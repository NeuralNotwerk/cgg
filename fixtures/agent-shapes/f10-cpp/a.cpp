#include <memory>
#include <optional>
struct Foo { int zorkle() { return 1; } };
struct Bar { int zorkle() { return 2; } };
void ctlPtr() { Foo* p = new Foo(); p->zorkle(); }
void ctlParam(Foo& f) { f.zorkle(); }
template <typename T> void ctlUntyped(T t) { t.zorkle(); }
void makeShared() { auto p = std::make_shared<Foo>(); p->zorkle(); }
void makeUnique() { auto p = std::make_unique<Foo>(); p->zorkle(); }
void declUnique() { std::unique_ptr<Foo> p(new Foo()); p->zorkle(); }
void declShared() { std::shared_ptr<Foo> p = std::make_shared<Foo>(); p->zorkle(); }
void paramShared(std::shared_ptr<Foo> f) { f->zorkle(); }
void paramConstRefShared(const std::shared_ptr<Foo>& f) { f->zorkle(); }
void paramOptional(std::optional<Foo> f) { f->zorkle(); }
void autoNew() { auto p = new Foo(); p->zorkle(); }
