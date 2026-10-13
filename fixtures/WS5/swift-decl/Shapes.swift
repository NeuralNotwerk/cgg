protocol Shape {
    func area() -> Double
}

class Square: Shape {
    func area() -> Double { return 1 }
}

class Circle: Shape {
    func area() -> Double { return 3 }
}

class Other {
    func area() -> Double { return 99 }
}

func total(s: Shape) -> Double {
    return s.area()
}
