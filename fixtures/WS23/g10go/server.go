package main

type Server struct{}

func (s *Server) zip() int { return 1 }

type Client struct{}

func (c *Client) zip() int { return 2 }

// receiver typing: s is *Server
func (s *Server) Zap() { s.zip() }

// value receiver, same name `c` typed differently in two methods
func (c Client) Ping() { c.zip() }

// same-package param, unqualified type
func use(q *Server, r Client) {
	q.zip()
	r.zip()
}

// grouped params `a, b *Server`
func both(a, b *Server) { a.zip(); b.zip() }

// near miss: untyped interface param stays unresolved / fan-out
func any(x interface{ zip() int }) { x.zip() }
