package main
func run(f Foo, o interface{}, p *Foo) {
	f.M()
	(f).M()
	Foo{A: 1}.M()
	(&Foo{}).M()
	o.(Foo).M()
	(*p).M()
}
