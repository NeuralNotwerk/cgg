struct Shape { virtual double area() = 0; virtual double perim() const = 0; virtual ~Shape() {} };
struct Sq : Shape { double area() override { return 1; } double perim() const override { return 4; } };
struct Ci : Shape { double area() override { return 3; } double perim() const override { return 6; } };
void run() { holder.item->area(); holder.item->perim(); }
