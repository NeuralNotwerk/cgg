package p

type Foo struct{ A int }

func (f Foo) Zorkle() int  { return 1 }
func (f *Foo) Pzork() int  { return 1 }

type Bar struct{ A int }

func (b Bar) Zorkle() int  { return 2 }
func (b *Bar) Pzork() int  { return 2 }

type Repo struct{}

func (r *Repo) Quux() int { return 1 }

type Other struct{}

func (o *Other) Quux() int { return 2 }

type Svc struct{ repo *Repo }

func NewFoo() *Foo { return &Foo{} }

func CtlVar()   { var f Foo; f.Zorkle() }
func CtlLit()   { f := Foo{A: 1}; f.Zorkle() }
func CtlCtor()  { f := NewFoo(); f.Pzork() }
func MultiVar() {
	var f Foo
	f.
		Zorkle()
}
func LitRecv()   { Foo{A: 1}.Zorkle() }
func AddrRecv()  { (&Foo{A: 1}).Pzork() }
func ParenVar()  { var f Foo; (f).Zorkle() }
func (s *Svc) CtlField() { s.repo.Quux() }
func (s *Svc) MultiField() {
	s.repo.
		Quux()
}
