package main

import "github.com/robfig/cron/v3"

type Runner struct{}

func (r *Runner) Start() {}

func getCronMap(name string) *cron.Cron { return nil }
func newRunner() (*Runner, error)       { return nil, nil }
func pick() func() *Runner              { return nil }

func add() {
	cron := getCronMap("x")
	cron.Start()
	getCronMap("y").Start()
	r, _ := newRunner()
	r.Start()
	f := pick()
	v := f()
	v.Start()
	m := make(map[string]int)
	_ = m
}
