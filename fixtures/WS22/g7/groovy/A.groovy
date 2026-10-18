interface Shape { double area() }
abstract class Base { abstract double perim() }
class Sq extends Base implements Shape { double area() { 1 } double perim() { 4 } }
class Ci extends Base implements Shape { double area() { 3 } double perim() { 6 } }
class Use { void run() { holder.item.area(); holder.item.perim() } }
