package pkgb

type Server struct{}

func (s *Server) zip() int { return 9 }

func local(s *Server) { s.zip() }
