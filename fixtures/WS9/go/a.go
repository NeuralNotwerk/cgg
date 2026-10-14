package main

type T struct{}

func F[X any](x X) X { return x }
func G[X any, Y any]() {}
func IsA[E error](err error) bool { return false }

type MyInt int

func caller(err error) {
	F[int](1)
	G[int, string]()
	IsA[*T](err)
	_ = MyInt(3)
	arr := []func(){}
	arr[0]()
	F(2)
}
