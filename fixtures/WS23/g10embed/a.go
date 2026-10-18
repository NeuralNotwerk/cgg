package main

type Base struct{}

func (b *Base) Hello() int { return 1 }

type Other struct{}

func (o *Other) Hello() int { return 2 }

type Server struct {
	*Base
	name string
}

func (s *Server) Run() { s.Hello() }

type Iface interface{ Hello() int }

func viaIface(i Iface) { i.Hello() }
