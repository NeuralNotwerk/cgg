package util

type T struct{}

func F() int { return 1 }

func (t *T) M() int { return 1 }

func Open(p string) (*T, error) { return &T{}, nil }
