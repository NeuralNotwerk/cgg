package main

import "fmt"

type alpha struct{}

func (x alpha) process() { fmt.Println("Alpha") }

func (x alpha) unusedOp() { fmt.Println("u") }
