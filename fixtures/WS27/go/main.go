package main

import "example.com/ws27/store"

type HandlerFunc func()

func pass(fs ...interface{}) {}

// Parameter.
func withParam(cb func()) { cb() }

// := local.
func withShort(chain []func()) {
	next := chain[0]
	next()
}

// range variable.
func withRange(hs []func()) {
	for _, h := range hs {
		h()
	}
}

// type-switch binding.
func withTypeSwitch(v interface{}) {
	switch f := v.(type) {
	case func():
		f()
	}
}

// select case binding.
func withSelect(ch chan func()) {
	select {
	case fn := <-ch:
		fn()
	}
}

// if initializer.
func withIf() {
	if run := pick(); run != nil {
		run()
	}
}

// var in a body.
func withVar() {
	var step func() = pick()
	step()
}

// named result.
func withResult() (build func()) {
	build = pick()
	build()
	return
}

// receiver.
func (work HandlerFunc) Serve() { work() }

// func literal parameter.
func withLiteral() {
	go func(work func()) { work() }(nil)
}

// Value positions through a parameter: no reference.
func passParam(cb func()) { pass(cb) }

// A parameter named like its type's package: `store.Get()` is a call
// through the value, typed `store.Store`, reached through the package.
func useStore(store store.Store) string { return store.Get() }

// Another parameter named like that package: `g`'s type was written in
// the signature, where no parameter is in scope.
func useStoreBeside(g store.Store, store int) { _ = store; g.Put("x") }

// Positive controls: the initializer sees the package function; after
// an inner block ends the package function is visible again; a local in
// a sibling block does not hide a value named before it.
func controls() {
	n := now()
	_ = n
	{
		later := 1
		_ = later
	}
	later()
	pass(helper)
	{
		helper := 2
		_ = helper
	}
}

func main() {
	withParam(nil)
	withShort(nil)
	withRange(nil)
	withTypeSwitch(nil)
	withSelect(nil)
	withIf()
	withVar()
	withResult()
	HandlerFunc(nil).Serve()
	withLiteral()
	passParam(nil)
	controls()
	useStore(nil)
	useStoreBeside(nil, 0)
}
