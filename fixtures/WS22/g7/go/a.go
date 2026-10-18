package a
type Shape interface { Area() float64 }
type Sq struct{}
func (s Sq) Area() float64 { return 1 }
type Ci struct{}
func (c Ci) Area() float64 { return 3 }
func run() { holder.item.Area() }
