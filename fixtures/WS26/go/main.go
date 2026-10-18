package main

import (
	"net/http"
	"sort"

	"example.com/fx/util"
)

type Server struct{ Name string }

func (s *Server) handle(w http.ResponseWriter, r *http.Request) {}
func (s *Server) neverUsed()                                    {}
func (s *Server) Name2() string                                 { return s.Name }

func less(i, j int) bool    { return i < j }
func onTick()               {}
func inTable()              {}
func keyed()                {}
func assigned()             {}
func returned() func()      { return returnedTarget }
func returnedTarget()       {}
func orphan()               {}          // positive control
func shadowed()             {}          // near-miss: only a parameter of this name is passed
func inString()             {}          // near-miss: only a string
func Field()                {}          // near-miss: only a struct key of this name
func local()                {}          // near-miss: only a local of this name is passed

type T struct{ Field int }

func run(fn func()) { fn() }

func useParam(shadowed func()) { run(shadowed) }

func (s *Server) routes(mux *http.ServeMux) {
	mux.Handle("/x", http.HandlerFunc(s.handle))
	println(s.Name)
}

func main() {
	xs := []int{3, 1}
	sort.Slice(xs, less)
	run(onTick)
	_ = []func(){inTable}
	_ = map[string]func(){"k": keyed}
	var f func()
	f = assigned
	_ = f
	_ = returned()
	_ = util.Format
	println("inString")
	_ = T{Field: 1}
	local := func() {}
	_ = local
	useParam(nil)
	(&Server{}).routes(http.NewServeMux())
}
