package util

type T struct{}

func F() int { return 2 }

func (t *T) M() int { return 2 }

func Open(p string) (*T, error) { return &T{}, nil }
