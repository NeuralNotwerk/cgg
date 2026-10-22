package impl

import "example.com/app/store"

// Satisfies Driver: value and pointer receivers.
type Mem struct{ base }

func (m Mem) Get(key string) (string, error) { return "", nil }
func (m *Mem) Put(key, value string) error    { return nil }

// Promoted Flush and Close via the embedded base.
type base struct{}

func (b *base) Flush() error { return nil }
func (b *base) Close() error { return nil }

// Missing Put: not a Driver.
type Half struct{}

func (h Half) Get(key string) (string, error) { return "", nil }

// Put with the wrong parameter count: not a Driver.
type Odd struct{}

func (o Odd) Get(key string) (string, error) { return "", nil }
func (o Odd) Put(key string) error            { return nil }

// fmt.Stringer, nobody in the tree calls it.
type Name struct{}

func (n Name) String() string { return "n" }

func Run() {
	s := &store.Store{}
	s.Lookup("k")
	_ = Mem{}
	_ = Half{}
	_ = Odd{}
	_ = Name{}
}

func main() { Run() }
