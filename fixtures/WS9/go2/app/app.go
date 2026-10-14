package app

import (
	"example.com/m/errs"
	"strings"
)

type H func(int)

var handlers = map[string]H{}

type T struct{ fns []func() }

func (t *T) Error() string { return "" }

func caller(err error, t *T, k string) {
	errs.IsA[*T](err)
	errs.As2[*T, *T](err, 1)
	errs.IsA[*errs.X](err)
	handlers[k](1)
	t.fns[0]()
	_ = strings.Map[int](nil, "")
	local := []func(int){}
	local[0](2)
}
