package main

import "example.com/app/internal/a/util"

func main() {
	util.F()
	t, err := util.Open("x")
	_ = err
	t.M()
	w := &util.T{}
	w.M()
}

func use(t *util.T) int {
	return t.M()
}
