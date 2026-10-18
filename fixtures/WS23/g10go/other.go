package main

type Third struct{}

func (t *Third) zip() int { return 3 }

// shadowing inner scope: s param is *Server, inner s := Third{}
func shadow(s *Server) {
	s.zip()
	{
		s := &Third{}
		s.zip()
	}
}

// closure parameter and if-initializer shadow the outer parameter
func shadow2(s *Server, xs []int) {
	f := func(s *Third) { s.zip() }
	_ = f
	if s := (&Third{}); s != nil {
		s.zip()
	}
	s.zip()
}

// lowercase same-package type
type store struct{}

func (st *store) load() int { return 0 }

func useStore(db *store) { db.load() }
