abstract class Shape {
  double area();
  String describe() => 'shape';
}

class Square extends Shape {
  @override
  double area() => 1;
}

class Circle implements Shape {
  @override
  double area() => 3;
  @override
  String describe() => 'circle';
}

class Other {
  double area() => 99;
}

double total(Shape s) {
  return s.area();
}
