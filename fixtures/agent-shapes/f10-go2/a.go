package p

type Foo struct{}

func (f Foo) Zorkle() int { return 1 }

type Bar struct{}

func (b Bar) Zorkle() int { return 2 }

func ValParam(f Foo)  { f.Zorkle() }
func PtrParam(f *Foo) { f.Zorkle() }
func Untyped(x any)   { x.(Foo).Zorkle() }
