struct Shape {
  virtual ~Shape() {}
  virtual double area() const = 0;
  virtual void draw() = 0;
};
