package p

type Box struct{}

func (b Box) CtlM(x int) int { return x }

func GenF[T any](x T) T { return x }
func GenF2[T any, U any](x T, u U) T { return x }
func CtlF(x int) int { return x }

func UseGen() {
	GenF[int](1)
	GenF2[int, string](1, "a")
}

func UseCtl() {
	b := Box{}
	CtlF(1)
	b.CtlM(2)
}
