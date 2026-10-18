interface IShape { double Area(); }
abstract class Base { public abstract double Perim(); public double Half() { return Perim() / 2; } }
class Sq : Base, IShape { public double Area() { return 1; } public override double Perim() { return 4; } }
class Ci : Base, IShape { public double Area() { return 3; } public override double Perim() { return 6; } }
class Use { void Run() { holder.item.Area(); holder.item.Perim(); } }
