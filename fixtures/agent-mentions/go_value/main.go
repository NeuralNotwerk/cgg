package main

import (
	"net/http"
	"sort"
	"strings"
)

func handler(w http.ResponseWriter, r *http.Request) {}

func lessFn(i, j int) bool { return i < j }

func mapFn(r rune) rune { return r }

func assignedFn() int { return 1 }

func returnedFn() int { return 2 }

func structFieldFn() int { return 3 }

func deferTarget() {}

func deadControl() {}

func calledControl() {}

type ops struct{ f func() int }

type T struct{}

func (t *T) methodValue() int { return 4 }

func makeFn() func() int { return returnedFn }

func main() {
	http.HandleFunc("/", handler)
	xs := []int{3, 1}
	sort.Slice(xs, lessFn)
	_ = strings.Map(mapFn, "abc")
	f := assignedFn
	o := ops{f: structFieldFn}
	t := &T{}
	g := t.methodValue
	defer deferTarget()
	calledControl()
	_, _, _, _ = f, o, g, makeFn()
}
