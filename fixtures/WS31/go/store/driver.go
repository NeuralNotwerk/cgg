package store

import "io"

type Driver interface {
	Get(key string) (string, error)
	Put(key, value string) error
}

type ClosingDriver interface {
	Driver
	io.Closer
	Flush() error
}

type Store struct {
	driver Driver
}

func (s *Store) Lookup(k string) (string, error) {
	return s.driver.Get(k)
}
