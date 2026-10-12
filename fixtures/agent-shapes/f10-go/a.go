package p

import "sync/atomic"

type Foo struct{}

func (f *Foo) Zorkle() int { return 1 }

type Bar struct{}

func (b *Bar) Zorkle() int { return 2 }

func CtlTyped(f *Foo)            { f.Zorkle() }
func CtlUntyped(x interface{})   { x.(*Foo).Zorkle() }
func SliceRange(xs []*Foo)       { for _, x := range xs { x.Zorkle() } }
func SliceIndex(xs []*Foo)       { xs[0].Zorkle() }
func MapIndex(m map[string]*Foo) { m["a"].Zorkle() }
func Atomic(p *atomic.Pointer[Foo]) { p.Load().Zorkle() }
func TypeAssertLocal(x interface{}) { f := x.(*Foo); f.Zorkle() }
func NewLocal()                  { f := new(Foo); f.Zorkle() }
func AddrLit()                   { f := &Foo{}; f.Zorkle() }
