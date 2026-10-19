class Counter {
  int count = 0;
  int get size => count;
  Counter();
  Counter.named();
  void handleTap() {}
  void unused() {}              // control: dead
  void build(List<int> xs) {
    final b = Button(onPressed: handleTap);
    xs.map(parse);
    final g = Counter.new;
    final h = Counter.named;
    print(b); print(g); print(h); print(size);
  }
}
class Button { Button({required Function onPressed}); }
int parse(int x) => x;
int count() => 0;               // near-miss: only the field `count` is read
void shadowed() {}              // near-miss: only a parameter of this name is passed
void orphan() {}                // control: dead
void invoke(Function f) => f();
void use(Function shadowed) { invoke(shadowed); }
void main() {
  Counter().build([1]);
  use(() {});
}
