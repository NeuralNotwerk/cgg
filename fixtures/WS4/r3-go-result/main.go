package main

import "example.com/r3/svc"

func main() {
	l := svc.NewLocal()
	l.Endpoint()
	p := svc.GetIdv()
	p.Verify()
	s, _ := svc.GetStore()
	s.Endpoint()
}
