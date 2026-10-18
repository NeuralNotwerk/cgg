interface Shape { double area(); default double twice() { return 2 * area(); } }
abstract class Base { abstract double perim(); double half() { return perim() / 2; } }
class Sq extends Base implements Shape { public double area() { return 1; } double perim() { return 4; } }
class Ci extends Base implements Shape { public double area() { return 3; } double perim() { return 6; } }
class Use { void run() { this.holder.item.area(); this.holder.item.perim(); } }
